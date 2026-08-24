//! 文件预分配与按偏移写入。各连接线程各自持有一个 File 句柄。

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

pub fn open_part(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(path)
}

/// 尽量把 .part 预分配到 `size`，减少下载过程中的碎片。失败不致命。
pub fn preallocate(file: &File, size: u64) {
    if size == 0 {
        return;
    }
    let _ = file.set_len(size);
}

pub fn write_at(file: &mut File, offset: u64, buf: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        let mut wrote = 0;
        while wrote < buf.len() {
            let n = file.write_at(&buf[wrote..], offset + wrote as u64)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "write_at 写了 0 字节"));
            }
            wrote += n;
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut wrote = 0;
        while wrote < buf.len() {
            let n = file.seek_write(&buf[wrote..], offset + wrote as u64)?;
            if n == 0 {
                return Err(io::Error::new(io::ErrorKind::WriteZero, "seek_write 写了 0 字节"));
            }
            wrote += n;
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(buf)
    }
}

/// 删除文件或目录。不存在就算成功。
pub fn remove_path(path: &Path) {
    if !path.exists() {
        return;
    }
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

pub fn remove_paths(paths: &[std::path::PathBuf]) {
    for p in paths {
        remove_path(p);
    }
}

/// 尽量送进系统废纸篓；失败再永久删除。
pub fn trash_paths(paths: &[std::path::PathBuf]) {
    for p in paths {
        if !p.exists() {
            continue;
        }
        if crate::platform::move_to_trash(p).is_err() {
            remove_path(p);
        }
    }
}

pub fn finalize_part(part: &Path, dest: &Path) -> io::Result<()> {
    if dest.exists() {
        std::fs::remove_file(dest)?;
    }
    std::fs::rename(part, dest)
}

/// 把若干段按顺序拼到 `dest`。HLS 用。
pub fn concat_files(parts: &[std::path::PathBuf], dest: &Path) -> io::Result<u64> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut out = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(dest)?;
    let mut total = 0u64;
    let mut buf = vec![0u8; 256 * 1024];
    for p in parts {
        let mut f = File::open(p)?;
        loop {
            let n = std::io::Read::read(&mut f, &mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            total += n as u64;
        }
    }
    out.flush()?;
    Ok(total)
}
