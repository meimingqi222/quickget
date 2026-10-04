//! 文件预分配与按偏移写入。各连接线程各自持有一个 File 句柄。

use crate::core::progress::Control;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "write_at 写了 0 字节",
                ));
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
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "seek_write 写了 0 字节",
                ));
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
/// 先处理文件再处理目录，避免目录因里面还有文件而删不掉。
pub fn trash_paths(paths: &[std::path::PathBuf]) {
    let mut dirs = Vec::new();
    for p in paths {
        if !p.exists() {
            continue;
        }
        if p.is_dir() {
            dirs.push(p);
            continue;
        }
        if crate::platform::move_to_trash(p).is_err() {
            remove_path(p);
        }
    }
    for p in dirs {
        if !p.exists() {
            continue;
        }
        if crate::platform::move_to_trash(p).is_err() {
            remove_path(p);
        }
    }
}

/// Create a unique temporary file beside `path`.
///
/// Keeping the temporary file in the destination directory makes the final
/// rename atomic on the platforms supported by the application. `create_new`
/// also prevents two download instances from sharing a temporary pathname.
pub(crate) fn create_temp_file(path: &Path) -> io::Result<(PathBuf, File)> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut stem = OsString::from(path.as_os_str());
    stem.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let temp = PathBuf::from(stem);
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    Ok((temp, file))
}

/// Replace `dest` with `source` without deleting an existing destination.
pub fn atomic_replace(source: &Path, dest: &Path) -> io::Result<()> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use winapi::shared::minwindef::{BOOL, DWORD};
        use winapi::um::winbase::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            REPLACEFILE_WRITE_THROUGH,
        };

        #[link(name = "kernel32")]
        extern "system" {
            fn ReplaceFileW(
                replaced: *const u16,
                replacement: *const u16,
                backup: *const u16,
                flags: DWORD,
                exclude: *mut std::ffi::c_void,
                reserved: *mut std::ffi::c_void,
            ) -> BOOL;
        }

        let source_w: Vec<u16> = source
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let dest_w: Vec<u16> = dest
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut last_error = None;
        for attempt in 0..5 {
            let result = unsafe {
                if dest.exists() {
                    if ReplaceFileW(
                        dest_w.as_ptr(),
                        source_w.as_ptr(),
                        std::ptr::null(),
                        REPLACEFILE_WRITE_THROUGH,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    ) != 0
                    {
                        return Ok(());
                    }
                    std::io::Error::last_os_error()
                } else if MoveFileExW(
                    source_w.as_ptr(),
                    dest_w.as_ptr(),
                    (MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) as DWORD,
                ) != 0
                {
                    return Ok(());
                } else {
                    std::io::Error::last_os_error()
                }
            };
            let transient = matches!(result.raw_os_error(), Some(5 | 32 | 33 | 170))
                || result.kind() == io::ErrorKind::PermissionDenied;
            last_error = Some(result);
            if !transient || attempt == 4 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20 * (attempt + 1)));
        }
        return Err(last_error
            .unwrap_or_else(|| io::Error::new(io::ErrorKind::Other, "Windows 文件替换失败")));
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(source, dest)
    }
}

/// Write bytes to a uniquely named sibling and publish them atomically.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let (temp, file) = create_temp_file(path)?;
    let result = (|| {
        let mut file = file;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        atomic_replace(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

pub fn finalize_part(part: &Path, dest: &Path) -> io::Result<()> {
    atomic_replace(part, dest)
}

/// 把若干段按顺序拼到 `dest`。HLS 用。
pub fn concat_files(parts: &[PathBuf], dest: &Path) -> io::Result<u64> {
    concat_files_with_control(parts, dest, None)
}

/// Like [`concat_files`], but checks pause/cancel before each copy chunk.
pub fn concat_files_with_control(
    parts: &[PathBuf],
    dest: &Path,
    ctrl: Option<&Control>,
) -> io::Result<u64> {
    let (temp, out) = create_temp_file(dest)?;
    let result = (|| {
        let mut out = out;
        let mut total = 0u64;
        let mut buf = vec![0u8; 256 * 1024];
        for p in parts {
            if ctrl.is_some_and(Control::interrupted) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "下载已暂停或取消",
                ));
            }
            let mut f = File::open(p)?;
            loop {
                if ctrl.is_some_and(Control::interrupted) {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "下载已暂停或取消",
                    ));
                }
                let n = std::io::Read::read(&mut f, &mut buf)?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n])?;
                total += n as u64;
            }
        }
        if ctrl.is_some_and(Control::interrupted) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "下载已暂停或取消",
            ));
        }
        out.flush()?;
        out.sync_all()?;
        drop(out);
        atomic_replace(&temp, dest)?;
        Ok(total)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qg-io-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn atomic_write_replaces_existing_without_intermediate_file() {
        let dir = temp_dir();
        let path = dir.join("state.json");
        std::fs::write(&path, b"old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn concat_failure_keeps_existing_destination() {
        let dir = temp_dir();
        let dest = dir.join("output.bin");
        let first = dir.join("one");
        let missing = dir.join("missing");
        std::fs::write(&dest, b"sentinel").unwrap();
        std::fs::write(&first, b"first").unwrap();

        let result = concat_files(&[first, missing], &dest);
        assert!(result.is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"sentinel");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn concat_publishes_complete_ordered_output() {
        let dir = temp_dir();
        let dest = dir.join("output.bin");
        let first = dir.join("one");
        let second = dir.join("two");
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();

        assert_eq!(concat_files(&[first, second], &dest).unwrap(), 11);
        assert_eq!(std::fs::read(&dest).unwrap(), b"firstsecond");
        let _ = std::fs::remove_dir_all(dir);
    }
}
