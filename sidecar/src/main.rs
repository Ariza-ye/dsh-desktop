//! Owns DSH's process tree. The desktop holds stdin open; EOF means its owner exited.
use std::{
    env,
    ffi::OsString,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

fn search_path() -> OsString {
    let mut paths = Vec::new();
    if let Some(executable) = env::var_os("DSH_EXECUTABLE") {
        if let Some(parent) = Path::new(&executable)
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
        {
            paths.push(parent.to_path_buf());
        }
    }
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    // Finder does not inherit the PATH from an interactive shell.
    paths.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    if let Some(home) = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        let home = PathBuf::from(home);
        for suffix in [
            ".local/bin",
            ".npm-global/bin",
            ".volta/bin",
            ".bun/bin",
            ".local/share/mise/shims",
            ".asdf/shims",
        ] {
            paths.push(home.join(suffix));
        }
        if let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) {
            let mut versions: Vec<_> = entries
                .flatten()
                .map(|entry| entry.path().join("bin"))
                .collect();
            versions.sort();
            versions.reverse();
            paths.extend(versions);
        }
        if cfg!(windows) {
            if let Some(appdata) = env::var_os("APPDATA") {
                paths.push(PathBuf::from(appdata).join("npm"));
            }
            if let Some(program_files) = env::var_os("ProgramFiles") {
                paths.push(PathBuf::from(program_files).join("nodejs"));
            }
        }
    }
    env::join_paths(paths).unwrap_or_else(|_| env::var_os("PATH").unwrap_or_default())
}

fn find_dsh(path: &std::ffi::OsStr) -> io::Result<PathBuf> {
    if let Some(executable) = env::var_os("DSH_EXECUTABLE") {
        // A configured path is authoritative: report a typo instead of silently using another DSH.
        let executable = PathBuf::from(executable);
        if executable.is_file() {
            return Ok(executable);
        }
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "DSH_EXECUTABLE 必须指向存在的 DSH 可执行文件",
        ));
    }
    let names: &[&str] = if cfg!(windows) {
        &["dsh.exe", "dsh.cmd", "dsh.bat"]
    } else {
        &["dsh"]
    };
    for directory in env::split_paths(path) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "未找到 DSH。请安装 npm install -g @deepseek-ai/dsh，或设置 DSH_EXECUTABLE 为绝对路径。",
    ))
}

struct ProcessTree {
    child: Child,
    #[cfg(windows)]
    job: windows_job::Job,
}

impl ProcessTree {
    fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let child = command.spawn()?;
        #[cfg(windows)]
        let job = match windows_job::Job::assign(&child) {
            Ok(job) => job,
            Err(error) => {
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            child,
            #[cfg(windows)]
            job,
        })
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let group = -(self.child.id() as i32);
            // DSH may create shells/workers. Signal the entire isolated group, even if DSH exited.
            unsafe {
                libc::kill(group, libc::SIGTERM);
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while Instant::now() < deadline {
                if unsafe { libc::kill(group, 0) } != 0 {
                    break;
                }
                let _ = self.child.try_wait();
                thread::sleep(Duration::from_millis(50));
            }
            unsafe {
                libc::kill(group, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        self.job.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run() -> Result<i32, Box<dyn std::error::Error>> {
    let stopping = Arc::new(AtomicBool::new(false));
    let signal = stopping.clone();
    ctrlc::set_handler(move || signal.store(true, Ordering::SeqCst))?;
    let owner = stopping.clone();
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut buffer = [0; 256];
        // Any input is a shutdown request; EOF also covers a desktop crash.
        let _ = input.read(&mut buffer);
        owner.store(true, Ordering::SeqCst);
    });

    let path = search_path();
    let executable = find_dsh(&path)?;
    let mut command = Command::new(executable);
    command
        .args(["web", "--no-open", "--host", "127.0.0.1", "--port", "0"])
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut tree = ProcessTree::spawn(&mut command)?;
    loop {
        if stopping.load(Ordering::SeqCst) {
            return Ok(0);
        }
        if let Some(status) = tree.child.try_wait()? {
            return Ok(status.code().unwrap_or(1));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("dsh-sidecar: {error}");
            1
        }
    };
    std::process::exit(code);
}

#[cfg(windows)]
mod windows_job {
    use std::{
        io,
        mem::{size_of, zeroed},
        os::windows::io::AsRawHandle,
        process::Child,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
    };

    pub struct Job(HANDLE);
    impl Job {
        pub fn assign(child: &Child) -> io::Result<Self> {
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let job = Self(handle);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                if SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                ) == 0
                    || AssignProcessToJobObject(handle, child.as_raw_handle()) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(job)
            }
        }
        pub fn terminate(&self) {
            unsafe {
                TerminateJobObject(self.0, 0);
            }
        }
    }
    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
