//! 下载编排：按协议把任务交给 HTTP / FTP / HLS / BT。

use crate::core::bt::{download_bt, BtJob};
use crate::core::ftp::{download_ftp, FtpJob};
use crate::core::hls::{default_hls_filename, download_hls, HlsJob};
use crate::core::http::{download_http, HttpJob};
use crate::core::model::Task;
use crate::core::progress::{Control, JobOutcome, LiveMeta, LiveProgress};
use crate::core::urlx::Protocol;
use std::sync::Arc;

pub fn run_task(
    task: &Task,
    ua: &str,
    progress: Arc<LiveProgress>,
    ctrl: Control,
    meta: LiveMeta,
) -> JobOutcome {
    crate::core::log::write(format_args!(
        "开始 {} {} -> {}",
        task.protocol.label(),
        task.url,
        task.dest_path().display()
    ));
    match task.protocol {
        Protocol::Magnet => download_bt(BtJob {
            url: &task.url,
            dest: &task.dest_path(),
            progress,
            meta,
            ctrl,
        }),
        Protocol::Unknown => JobOutcome::Failed("无法识别的协议".into()),
        Protocol::Ftp => download_ftp(FtpJob {
            url: &task.url,
            dest: &task.dest_path(),
            part: &task.part_path(),
            progress,
            ctrl,
        }),
        Protocol::Hls => {
            let dest = {
                let p = task.dest_path();
                if p.extension().and_then(|e| e.to_str()) == Some("m3u8") {
                    p.with_file_name(default_hls_filename(&task.url))
                } else {
                    p
                }
            };
            download_hls(HlsJob {
                url: &task.url,
                dest: &dest,
                connections: task.connections,
                ua,
                referer: task.referer.as_deref(),
                cookies: task.cookies.as_deref(),
                progress,
                ctrl,
            })
        }
        Protocol::Http => download_http(HttpJob {
            url: &task.url,
            dest: &task.dest_path(),
            part: &task.part_path(),
            meta_path: &task.meta_path(),
            connections: task.connections,
            ua,
            referer: task.referer.as_deref(),
            cookies: task.cookies.as_deref(),
            progress,
            ctrl,
            max_part_size: task.max_part_size,
        }),
    }
}

#[cfg(test)]
mod http_live_tests {
    use super::*;
    use crate::core::http::build_client;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::thread;

    /// 一个认 Range 的迷你 HTTP 服务，专门用来证明多连接能把文件拼回去。
    fn spawn_range_server(body: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).ok();
        let handle = thread::spawn(move || {
            let start = std::time::Instant::now();
            while start.elapsed() < std::time::Duration::from_secs(8) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                            .ok();
                        let mut buf = [0u8; 4096];
                        let mut req = Vec::new();
                        loop {
                            match stream.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    req.extend_from_slice(&buf[..n]);
                                    if req.windows(4).any(|w| w == b"\r\n\r\n") {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        let req_s = String::from_utf8_lossy(&req);
                        let range = req_s.lines().find_map(|l| {
                            let l = l.to_ascii_lowercase();
                            l.strip_prefix("range: bytes=")
                                .map(|s| s.trim().to_string())
                        });
                        let (status, slice, cr) = if let Some(r) = range {
                            let r = r.trim_end_matches('\r').to_string();
                            let (a, b) = r.split_once('-').unwrap_or(("0", ""));
                            let start: usize = a.parse().unwrap_or(0);
                            let end: usize = if b.is_empty() {
                                body.len().saturating_sub(1)
                            } else {
                                b.parse().unwrap_or(body.len().saturating_sub(1))
                            };
                            let end = end.min(body.len().saturating_sub(1));
                            let start = start.min(end);
                            (
                                "206 Partial Content",
                                &body[start..=end],
                                Some(format!(
                                    "Content-Range: bytes {start}-{end}/{}\r\n",
                                    body.len()
                                )),
                            )
                        } else {
                            ("200 OK", body.as_slice(), None)
                        };
                        let mut head = format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n",
                            slice.len()
                        );
                        if let Some(cr) = cr {
                            head.push_str(&cr);
                        }
                        head.push_str("\r\n");
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(slice);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        (format!("http://{addr}/file.bin"), handle)
    }

    #[test]
    fn multi_range_round_trips_bytes() {
        let payload: Vec<u8> = (0..50_000u32).map(|i| (i % 251) as u8).collect();
        let expected = payload.clone();
        let (url, server) = spawn_range_server(payload);

        // 先确认探活通了，再跑引擎。
        let client = build_client("quickget-test").unwrap();
        let resp = client
            .get(&url)
            .header("Range", "bytes=0-0")
            .send()
            .unwrap();
        assert!(resp.status().as_u16() == 206 || resp.status().is_success());
        drop(resp);

        let dir = std::env::temp_dir().join(format!("quickget-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("file.bin");
        let task = Task {
            id: "t1".into(),
            url: url.clone(),
            filename: "file.bin".into(),
            save_dir: dir.clone(),
            protocol: Protocol::Http,
            status: crate::core::model::TaskStatus::Downloading,
            size: expected.len() as u64,
            downloaded: 0,
            connections: 8,
            error: None,
            created_at: 0,
            finished_at: None,
            referer: None,
            cookies: None,
            user_agent: None,
            files: Vec::new(),
            output_dir: None,
            max_part_size: None,
            cleanup_paths: None,
            cleanup_cookies: None,
            peers: Default::default(),
        };
        let progress = LiveProgress::new(0, expected.len() as u64);
        let ctrl = Control {
            stop: Arc::new(AtomicBool::new(false)),
            pause: Arc::new(AtomicBool::new(false)),
        };
        let outcome = run_task(&task, "quickget-test", progress, ctrl, LiveMeta::new());
        let _ = server.join();
        match outcome {
            JobOutcome::Completed { size, path } => {
                assert_eq!(size, expected.len() as u64);
                let got = std::fs::read(&path).unwrap();
                assert_eq!(got, expected);
            }
            other => panic!("unexpected {other:?}"),
        }
        let _ = std::fs::remove_dir_all(dir);
        let _ = dest;
        let _ = PathBuf::new();
    }
}
