use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Serialize)]
struct Status {
    phase: String,
    message: String,
    logs: VecDeque<String>,
}

struct Inner {
    status: Status,
    child: Option<CommandChild>,
    generation: u64,
    local_url: Option<tauri::Url>,
    dsh_origin: Option<String>,
    exiting: bool,
}

struct DshState(Mutex<Inner>);

impl Default for DshState {
    fn default() -> Self {
        Self(Mutex::new(Inner {
            status: Status {
                phase: "starting".into(),
                message: "正在启动本地 DSH 服务…".into(),
                logs: VecDeque::new(),
            },
            child: None,
            generation: 0,
            local_url: None,
            dsh_origin: None,
            exiting: false,
        }))
    }
}

#[tauri::command]
fn dsh_status(state: tauri::State<'_, DshState>) -> Status {
    state.0.lock().unwrap().status.clone()
}

// DSH prints an authenticated URL, possibly with a query/fragment token. Keep it intact
// for navigation, but never include the URL in diagnostics or grant the page remote IPC.
fn announced_url(line: &str) -> Option<tauri::Url> {
    let candidate = line.trim().strip_prefix("dsh web: ")?;
    let url = tauri::Url::parse(candidate).ok()?;
    (url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port().is_some_and(|port| port != 0)
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}

fn show_failure(app: &tauri::AppHandle, generation: u64, message: String) {
    let state = app.state::<DshState>();
    let local_url = {
        let mut inner = state.0.lock().unwrap();
        if inner.generation != generation || inner.exiting || inner.status.phase == "stopping" {
            return;
        }
        inner.status.phase = "failed".into();
        inner.status.message = message;
        if let Some(child) = inner.child.as_mut() {
            let _ = child.write(b"shutdown\n");
        }
        inner.local_url.clone()
    };
    if let (Some(window), Some(url)) = (app.get_webview_window("main"), local_url) {
        let _ = window.navigate(url);
    }
}

fn start_dsh(app: &tauri::AppHandle) -> Result<(), String> {
    eprintln!("Starting DSH sidecar");
    let state = app.state::<DshState>();
    let mut inner = state.0.lock().unwrap();
    if inner.child.is_some() || inner.exiting {
        return Err("DSH 正在运行或退出，请稍后重试。".into());
    }
    inner.generation += 1;
    let generation = inner.generation;
    inner.dsh_origin = None;
    inner.status = Status {
        phase: "starting".into(),
        message: "正在启动本地 DSH 服务…".into(),
        logs: VecDeque::new(),
    };
    let spawned = app
        .shell()
        .sidecar("dsh-sidecar")
        .and_then(|cmd| cmd.spawn());
    let (mut events, child) = match spawned {
        Ok(result) => result,
        Err(error) => {
            inner.status.phase = "failed".into();
            inner.status.message = format!("无法启动 Sidecar：{error}");
            eprintln!("{}", inner.status.message);
            return Err(inner.status.message.clone());
        }
    };
    inner.child = Some(child);
    drop(inner);

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let timeout = tokio::time::sleep(STARTUP_TIMEOUT);
        tokio::pin!(timeout);
        let mut ready = false;
        loop {
            tokio::select! {
                _ = &mut timeout, if !ready => {
                    show_failure(&handle, generation, "DSH 启动超时（120 秒），请查看诊断信息后重试。".into());
                    // Continue consuming events so shutdown can reap the sidecar.
                    ready = true;
                }
                event = events.recv() => {
                    match event {
                        Some(CommandEvent::Stdout(bytes)) | Some(CommandEvent::Stderr(bytes)) => {
                            let line = String::from_utf8_lossy(&bytes).into_owned();
                            let url = announced_url(&line);
                            {
                                let state = handle.state::<DshState>();
                                let mut inner = state.0.lock().unwrap();
                                if inner.generation != generation { return; }
                                if inner.status.logs.len() >= 80 { inner.status.logs.pop_front(); }
                                inner.status.logs.push_back(if line.contains("dsh web: http") {
                                    "DSH 已输出本地访问地址（认证信息已隐藏）".into()
                                } else { line });
                            }
                            if !ready {
                                if let Some(url) = url {
                                    let healthy = match reqwest::Client::builder()
                                        .no_proxy()
                                        .redirect(reqwest::redirect::Policy::none())
                                        .timeout(Duration::from_secs(5)).build() {
                                        Ok(client) => client.get(url.clone()).send().await
                                            .is_ok_and(|response| response.status().is_success() || response.status().is_redirection()),
                                        Err(_) => false,
                                    };
                                    if !healthy {
                                        show_failure(&handle, generation, "DSH 已输出地址，但本地 HTTP 服务未通过就绪检查。".into());
                                        ready = true;
                                        continue;
                                    }
                                    let state = handle.state::<DshState>();
                                    {
                                        let mut inner = state.0.lock().unwrap();
                                        if inner.generation != generation || inner.status.phase != "starting" { continue; }
                                        inner.dsh_origin = Some(url.origin().ascii_serialization());
                                        inner.status.phase = "running".into();
                                        inner.status.message = "DSH 服务已就绪".into();
                                    }
                                    ready = true;
                                    if let Some(window) = handle.get_webview_window("main") {
                                        if let Err(error) = window.navigate(url) {
                                            show_failure(&handle, generation, format!("无法打开 DSH Web：{error}"));
                                        }
                                    }
                                }
                            }
                        }
                        Some(CommandEvent::Error(error)) => {
                            show_failure(&handle, generation, format!("Sidecar 进程通信失败：{error}"));
                        }
                        Some(CommandEvent::Terminated(_)) | None => {
                            let failed = {
                                let state = handle.state::<DshState>();
                                let mut inner = state.0.lock().unwrap();
                                if inner.generation != generation { return; }
                                inner.child = None;
                                !inner.exiting && inner.status.phase != "stopping" && inner.status.phase != "failed"
                            };
                            if failed {
                                show_failure(&handle, generation, "DSH 已意外退出，请查看诊断信息后重试。".into());
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    });
    Ok(())
}

fn stop_dsh(app: &tauri::AppHandle, exiting: bool) {
    let state = app.state::<DshState>();
    {
        let mut inner = state.0.lock().unwrap();
        inner.exiting |= exiting;
        inner.status.phase = "stopping".into();
        if let Some(child) = inner.child.as_mut() {
            let _ = child.write(b"shutdown\n");
        } else {
            return;
        }
    }
    // Sidecar gives the process group two seconds to shut down gracefully.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if state.0.lock().unwrap().child.is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let child = state.0.lock().unwrap().child.take();
    if let Some(child) = child {
        let _ = child.kill();
    }
}

#[tauri::command]
async fn restart_dsh(app: tauri::AppHandle) -> Result<(), String> {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || stop_dsh(&handle, false))
        .await
        .map_err(|error| error.to_string())?;
    start_dsh(&app)
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(DshState::default())
        .invoke_handler(tauri::generate_handler![dsh_status, restart_dsh])
        .on_window_event(|window, event| {
            #[cfg(target_os = "macos")]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    // Match Cmd+H: keep the window and DSH alive, and hide the app.
                    if let Err(error) = window.app_handle().hide() {
                        eprintln!("无法隐藏 DSH Desktop：{error}");
                    }
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (window, event);
        })
        .setup(|app| {
            eprintln!("Creating DSH Desktop window");
            let handle = app.handle().clone();
            let guard = handle.clone();
            let window =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("DSH Desktop")
                    .inner_size(1280.0, 860.0)
                    .min_inner_size(800.0, 600.0)
                    .on_navigation(move |url| {
                        let state = guard.state::<DshState>();
                        let inner = state.0.lock().unwrap();
                        let origin = url.origin().ascii_serialization();
                        inner
                            .local_url
                            .as_ref()
                            .is_some_and(|local| local.origin() == url.origin())
                            || inner.dsh_origin.as_ref() == Some(&origin)
                            || (inner.local_url.is_none()
                                && (url.scheme() == "tauri"
                                    || url.host_str() == Some("tauri.localhost")
                                    || (url.host_str() == Some("127.0.0.1")
                                        && url.port() == Some(1420))))
                    })
                    .build()?;
            let local_url = window.url()?;
            handle.state::<DshState>().0.lock().unwrap().local_url = Some(local_url);
            // A startup error stays visible in the local window and can be retried.
            let _ = start_dsh(&handle);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("无法初始化 DSH Desktop");
    app.run(|app, event| match event {
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            // Dock reopen restores both Cmd+H and close-button hiding.
            let _ = app.show();
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }
        RunEvent::ExitRequested { .. } => stop_dsh(app, true),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::announced_url;

    #[test]
    fn preserves_authentication_in_announced_url() {
        let url =
            announced_url("dsh web: http://127.0.0.1:34567/?token=secret#auth=value").unwrap();
        assert_eq!(url.query(), Some("token=secret"));
        assert_eq!(url.fragment(), Some("auth=value"));
    }

    #[test]
    fn rejects_non_local_and_unrelated_output() {
        for line in [
            "dsh web: http://example.com:1234/",
            "dsh web: http://127.0.0.1.evil.test:1234/",
            "dsh web: http://user@127.0.0.1:1234/",
            "dsh web: http://127.0.0.1:0/",
            "dsh web: opening the default browser",
            "unrelated http://127.0.0.1:1234/",
        ] {
            assert!(announced_url(line).is_none(), "{line}");
        }
    }
}
