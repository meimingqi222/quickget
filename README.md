# QuickGet

[English](README.md) | [中文说明](README_zh.md)

Native, multi-connection download manager. Rust + GPUI, no WebView, no Electron.

HTTP / HTTPS / FTP / HLS / BitTorrent. Segmented downloads across multiple connections, with resume.

## Why

Downie is built for grabbing video from web pages. Thunder was built to saturate a link. QuickGet is a general-purpose downloader: paste a standard URL, fetch it over several TCP connections, save it to your Downloads folder. Not a browser. Not a media suite.

## Features

- **HTTP/HTTPS** multi-connection Range download. Default 16 connections, fewer for small files.
- **Resume**. Sidecar `.qg.json` plus a `.part` file. Quit mid-way, come back, it continues.
- **FTP** with REST resume.
- **HLS (m3u8)** parallel segment fetch, concatenated to `.ts`. Encrypted playlists are refused with a clear message.
- **BitTorrent**. Magnet links and `.torrent` files (HTTP URL or local path). DHT, trackers, incoming TCP/uTP peers, UPnP. Resume from partial files. Each torrent is stored in its own folder; deleting the task moves that folder to the trash.
- **Queue**. Cap concurrent tasks (default 3). The rest wait.
- **Global download limit**. HTTP/HTTPS, FTP, and HLS share one bandwidth cap, adjustable live in Settings.
- **Clipboard watch**. Copy a URL, the app asks if you want to download it.
- **CLI**. `quickget <url>` downloads without opening the window.
- **Bilingual** 中文 / English, follows the OS on first launch.
- **Chromium extension**. Right-click a link to send it here with cookies. Browser downloads are not hijacked by default.

## Chrome / Edge / Brave / Arc extension

1. Launch QuickGet once (or run `quickget --install-host`) so the native messaging host is registered for Chromium-based browsers.
2. Open `chrome://extensions`, enable Developer mode, load the `extension/` folder.
3. Right-click a link and choose the QuickGet item. The toolbar button sends the current page.

The extension ID is pinned to `agkijomhcpkgagodkkjknfnocfnalmcc`. Hijacking Chrome's own download list is off by default; turn it on in the popup if you want it.

## Build

```bash
cargo test
cargo run
cargo build --release
```

macOS bundle:

```bash
cargo install cargo-bundle
cargo bundle --release
```

## CLI

```bash
quickget https://example.com/file.zip
quickget --dir ~/Movies --connections 32 https://cdn.example/a.m3u8
quickget --limit 1024 https://example.com/file.zip # 1024 KiB/s
quickget "magnet:?xt=urn:btih:..."
quickget ubuntu-24.04.iso.torrent
quickget --gui
quickget --install-host
```

## Stack

Same family as [QuickCleaner](https://github.com/meimingqi222/quick-cleaner): Rust, GPUI 0.2, Material-style light UI, `reqwest` (rustls, HTTP/1.1) for HTTP, `librqbit` for BitTorrent.
