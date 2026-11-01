# DSH Desktop

基于 **Tauri 2 + Rust Sidecar + DSH Web** 的桌面应用。打开应用时自动启动 DSH，服务就绪后在原生 WebView 内打开 DSH Web；macOS 点击窗口关闭按钮时隐藏应用（与 `Cmd+H` 一致），DSH 继续运行，点击 Dock 图标即可恢复窗口；使用 `Cmd+Q` 或菜单退出应用时，自动关闭本次启动的 DSH 及其子进程。

```text
DSH Desktop（Tauri 2）
  ├─ 本地启动页：启动进度、错误诊断、重新启动
  └─ dsh-sidecar（随应用打包的 Rust 可执行文件）
       └─ dsh web --no-open --host 127.0.0.1 --port 0
            └─ DSH Web → 原生 WebView
```

## 环境准备

- Node.js 20.19+ 或 22.12+（也需满足所安装 DSH 的 Node.js 要求）。
- Rust stable，以及 [Tauri 2 系统依赖](https://v2.tauri.app/start/prerequisites/)。macOS 需要 Xcode Command Line Tools。
- 本机安装 DSH：

```bash
npm install -g @deepseek-ai/dsh
# 确认当前版本支持 --no-open、--host 和 --port 0
dsh web --help
```

应用包含 Sidecar，但**目前不内置 Node.js 和 DSH npm 包**。运行安装后的桌面应用也需要本机已经安装 DSH。已使用 `@deepseek-ai/dsh@0.2.1-rc.2` 验证；DSH 保持使用自身的配置、认证信息与 profile，应用不改写它们。首次初始化 DSH profile 可能需要联网。

## 开发与打包

```bash
npm ci
npm run dev
```

开发命令会先编译 Sidecar，再启动 Vite 和 Tauri。修改 `sidecar/` 后需要重新执行 `npm run dev`。单独运行 `npm run dev:web` 只用于预览启动页，无法调用桌面命令。

```bash
npm run build
```

打包命令自动构建当前目标平台的 Sidecar 和前端，再生成桌面安装包。输出在 `src-tauri/target/release/bundle/`。macOS 发布所需的签名和公证需自行配置。

交叉构建示例：

```bash
rustup target add x86_64-apple-darwin
npm run tauri -- build --target x86_64-apple-darwin
```

Sidecar 构建脚本会读取 Tauri 提供的 `TAURI_ENV_TARGET_TRIPLE`，生成 `src-tauri/binaries/dsh-sidecar-<target-triple>`（Windows 带 `.exe`）。跨平台安装包需要相应平台的工具链；当前脚本不支持 macOS universal 合并构建，请分别构建 arm64 和 x86_64。

## 指定 DSH 路径

默认从 `PATH` 和常见安装目录寻找 DSH，包含 Homebrew、Volta、nvm、mise/asdf shims 和用户 npm bin，兼容 macOS 从 Finder 启动时的精简 `PATH`。自定义安装可通过绝对路径指定：

```bash
DSH_EXECUTABLE=/opt/homebrew/bin/dsh npm run dev
```

`DSH_EXECUTABLE` 需要在启动桌面应用的进程环境中设置；项目不自动加载 `.env`。DSH 的环境变量（例如 `DSH_HOME`）会传递给子进程。DSH Web 绑定本机回环地址，使用系统分配的空闲端口。

## 生命周期与边界

- Tauri 使用 `tauri-plugin-shell` 的 Rust API 启动打包的 Sidecar，不向 Web 页面开放 shell 执行权限。
- Sidecar 启动 DSH；桌面端解析 DSH 输出的带认证信息的地址，检查 HTTP 就绪后保留完整地址进行导航。支持 DSH 通过 302 跳转换取 Cookie 的认证流程。
- 启动超过 120 秒、找不到 DSH 或 DSH 意外退出时，返回本地启动页，显示最近 80 行诊断并提供重试。已知启动地址中的令牌会从诊断中隐藏；DSH 其他插件自行输出的内容不保证脱敏。
- macOS 关闭窗口时保留窗口与当前页面状态并隐藏整个应用；点击 Dock 图标恢复窗口。`Cmd+Q` 和菜单“退出”仍执行真正退出。
- 退出时先向 Sidecar stdin 写入关闭请求；桌面进程崩溃或被强制关闭后，stdin EOF 同样触发清理。
- Unix 使用独立进程组，先发送 SIGTERM，最多等待 2 秒，再发送 SIGKILL。Windows 使用 Job Object 管理进程树。
- 仅管理当前应用启动的进程，不复用或终止用户另外启动的 DSH。多个应用实例各自使用独立端口。
- DSH Web 没有远程 Tauri IPC 授权，主窗口导航仅允许本地启动页和本次启动的 DSH origin。外部链接/新窗口、系统托盘、自动更新尚未接入。
- Unix 中主动脱离进程组的后代进程不在清理范围内；单独对 Sidecar 发送 SIGKILL 无法执行其退出清理。

## 验证

```bash
npm run build:sidecar
npm run build:web
cargo check --locked --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml
# Unix 上模拟 DSH：校验启动参数、HTTP 就绪、退出/EOF/SIGTERM 清理以及缺失文件错误
python3 scripts/smoke-sidecar.py
# 同时启动本机真实 DSH，验证认证和退出；会使用当前用户的 DSH profile
python3 scripts/smoke-sidecar.py --real
```

模拟测试会创建忽略 SIGTERM 的后代进程，确认强制清理后相应端口关闭。Windows 生命周期实现仍需在 Windows 环境验证。

## 目录

```text
src/                         本地启动页（TypeScript + CSS）
src-tauri/src/lib.rs          Tauri 启动、就绪检查、导航与退出管理
src-tauri/tauri.conf.json     Tauri 2 配置与 Sidecar 打包声明
src-tauri/capabilities/       本地窗口权限
sidecar/src/main.rs           DSH 查找与进程树生命周期管理
scripts/build-sidecar.mjs     按 Rust target 编译、复制 Sidecar
scripts/smoke-sidecar.py      进程生命周期集成测试
assets/icon.png              应用图标源文件
```
