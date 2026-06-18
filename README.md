# ahhhh mmt

轻量 Windows Tauri 应用：文字、语音转写和图片先保存到本地 SQLite，再自动追加到指定 Notion 页面。

## 开发运行

1. 安装 Rust、Microsoft C++ Build Tools 和 WebView2。
2. 在本目录执行 `npm install`。
3. 执行 `npm run dev`。

若 Cargo 提示通过 `127.0.0.1` 连接 crates.io 失败，请先关闭失效的代理环境变量或启动对应代理，再重新执行。

首次打开详情页后，填写 Notion 集成密钥和目标页面标识，并在 Notion 中将目标页面共享给该集成。

## 数据位置

记录数据库和压缩图片保存在 `%APPDATA%\com.verba-vista.inspiration-inbox`。Notion 密钥保存在 Windows 凭据管理器。
