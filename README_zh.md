# QuickGet

[English](README.md) | [中文说明](README_zh.md)

原生多连接下载器。Rust + GPUI，无 WebView，无 Electron。

支持 HTTP / HTTPS / FTP / HLS。多连接分段下载，断点续传。

## 定位

Downie 侧重网页视频抓取。迅雷侧重把带宽打满。QuickGet 面向通用文件下载：粘贴任意标准链接，多连接分段拉取，保存到下载目录。不是浏览器，也不是影音套件。

## 功能

- **HTTP/HTTPS** 多连接 Range 下载。默认 16 连接，小文件自动降低连接数。
- **断点续传**。`.qg.json` 元数据 + `.part` 临时文件。中途退出后可继续。
- **FTP**，支持 REST 续传。
- **HLS (m3u8)** 并行下载分片并拼接为 `.ts`。加密流会明确提示暂不支持。
- **队列**。同时下载数默认 3，超出部分排队。
- **剪贴板监听**。复制链接后可一键加入队列。
- **命令行**。`quickget <url>` 无需打开窗口。
- **中英双语**，首次启动跟随系统语言。
- **Chromium 扩展**。右键链接投递到 QuickGet，并带上登录 Cookie。默认不接管浏览器下载。

磁力链接可识别，当前版本暂不下载。

## Chrome / Edge / Brave / Arc 扩展

1. 启动一次 QuickGet（或运行 `quickget --install-host`），会把 Native Messaging 清单写到各 Chromium 换壳目录。
2. 打开 `chrome://extensions`，打开「开发者模式」，加载 `extension/` 目录。
3. 在网页链接上右键「用 QuickGet 下载链接」。工具栏按钮可下载当前页。

扩展 ID 已固定为 `agkijomhcpkgagodkkjknfnocfnalmcc`。接管浏览器自身下载默认关闭，可在弹窗里打开。

## 编译

```bash
cargo test
cargo run
cargo build --release
```

macOS 打包：

```bash
cargo install cargo-bundle
cargo bundle --release
```

## 命令行

```bash
quickget https://example.com/file.zip
quickget --dir ~/Movies --connections 32 https://cdn.example/a.m3u8
quickget --gui
quickget --install-host
```

## 技术栈

与 [QuickCleaner](https://github.com/meimingqi222/quick-cleaner) 相同：Rust、GPUI 0.2、浅色 Material 风格界面。传输层使用 `reqwest`（rustls，HTTP/1.1 多连接）。
