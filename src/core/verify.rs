//! 落盘校验：续传元数据不能比磁盘更乐观，zip 必须有头、有目录。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub fn read_at(path: &Path, offset: u64, n: usize) -> Option<Vec<u8>> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    if offset >= len {
        return None;
    }
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; n];
    let got = f.read(&mut buf).ok()?;
    buf.truncate(got);
    if buf.is_empty() {
        None
    } else {
        Some(buf)
    }
}

pub fn is_all_zero(buf: &[u8]) -> bool {
    !buf.is_empty() && buf.iter().all(|&b| b == 0)
}

pub fn looks_zip_local(buf: &[u8]) -> bool {
    buf.len() >= 4 && buf[0] == b'P' && buf[1] == b'K' && buf[2] == 3 && buf[3] == 4
}

pub fn filename_looks_zip(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".zip")
}

/// zip 中央目录在文件尾部。注释最长 64KB，所以只扫末尾一段。
pub fn zip_has_eocd(path: &Path) -> bool {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let len = match f.metadata() {
        Ok(m) => m.len(),
        Err(_) => return false,
    };
    if len < 22 {
        return false;
    }
    let take = len.min(65_535 + 22 + 256);
    let start = len - take;
    if f.seek(SeekFrom::Start(start)).is_err() {
        return false;
    }
    let mut buf = vec![0u8; take as usize];
    let n = match f.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return false,
    };
    buf[..n].windows(4).any(|w| w == b"PK\x05\x06")
}

/// 对照磁盘，把续传里「声称已写完」但实际是空洞的段打回 0。
pub fn trusted_done(path: &Path, start: u64, done: u64, inspect_zip_header: bool) -> u64 {
    if done == 0 {
        return 0;
    }
    if !path.exists() {
        return 0;
    }
    let Some(head) = read_at(path, start, 32) else {
        return 0;
    };
    if inspect_zip_header && start == 0 && !looks_zip_local(&head) {
        return 0;
    }
    if is_all_zero(&head) {
        // 文件开头全零几乎一定是预分配空洞，不能当已完成。
        if start == 0 {
            return 0;
        }
        let mid = start + done / 2;
        match read_at(path, mid, 32) {
            Some(m) if is_all_zero(&m) => return 0,
            None => return 0,
            _ => {}
        }
    }
    done
}

pub fn verify_finished(path: &Path, expected_size: u64, filename: &str) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("读不到成品：{e}"))?;
    if expected_size > 0 && meta.len() < expected_size {
        return Err(format!(
            "文件长度不足：已有 {}，应为 {}",
            meta.len(),
            expected_size
        ));
    }
    let named_zip = filename_looks_zip(filename);
    let head = read_at(path, 0, 4).unwrap_or_default();
    if named_zip || looks_zip_local(&head) {
        if !looks_zip_local(&head) {
            return Err("文件开头不是 zip（常见原因：分段空洞，预分配的零把开头占了）".into());
        }
        if !zip_has_eocd(path) {
            return Err("zip 目录缺失，文件不完整，不会改成成品名".into());
        }
    }
    if is_all_zero(&head) {
        return Err("文件开头全是空字节，不能当完成品".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp_file(bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qg-verify-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.bin");
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn cleanup(p: &std::path::Path) {
        if let Some(d) = p.parent() {
            let _ = std::fs::remove_dir_all(d);
        }
    }

    #[test]
    fn hole_at_start_is_not_trusted() {
        let mut data = vec![0u8; 1024];
        data[512] = 1;
        let p = temp_file(&data);
        assert_eq!(trusted_done(&p, 0, 1024, false), 0);
        cleanup(&p);
    }

    #[test]
    fn zip_header_required_when_asked() {
        let p = temp_file(&[0, 1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(trusted_done(&p, 0, 8, true), 0);
        cleanup(&p);
    }

    #[test]
    fn real_data_is_trusted() {
        let mut data = vec![7u8; 1024];
        data[0] = b'P';
        data[1] = b'K';
        data[2] = 3;
        data[3] = 4;
        let p = temp_file(&data);
        assert_eq!(trusted_done(&p, 0, 1024, true), 1024);
        cleanup(&p);
    }

    #[test]
    fn missing_part_file_is_zero() {
        let p = std::env::temp_dir().join("qg-no-such-part-xyz");
        assert_eq!(trusted_done(&p, 0, 99, false), 0);
    }

    #[test]
    fn finished_zip_needs_header_and_eocd() {
        let mut data = vec![0u8; 128];
        data[0..4].copy_from_slice(b"PK\x03\x04");
        data[120..124].copy_from_slice(b"PK\x05\x06");
        let p = temp_file(&data);
        verify_finished(&p, 128, "a.zip").unwrap();
        cleanup(&p);
    }

    #[test]
    fn finished_zip_rejects_zero_header() {
        let mut data = vec![0u8; 128];
        data[120..124].copy_from_slice(b"PK\x05\x06");
        let p = temp_file(&data);
        assert!(verify_finished(&p, 128, "a.zip").is_err());
        cleanup(&p);
    }

    #[test]
    fn finished_zip_rejects_missing_eocd() {
        let mut data = vec![1u8; 128];
        data[0..4].copy_from_slice(b"PK\x03\x04");
        let p = temp_file(&data);
        assert!(verify_finished(&p, 128, "a.zip").is_err());
        cleanup(&p);
    }

    #[test]
    fn non_zip_zero_header_rejected() {
        let p = temp_file(&[0, 0, 0, 0, 1, 2, 3, 4]);
        assert!(verify_finished(&p, 8, "a.bin").is_err());
        cleanup(&p);
    }

    #[test]
    fn non_zip_with_data_ok() {
        let p = temp_file(b"not a zip but real bytes");
        verify_finished(&p, 10, "a.bin").unwrap();
        cleanup(&p);
    }
}
