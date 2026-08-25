//! FTP 单连接下载，支持 REST 续传。

use crate::core::io::{finalize_part, open_part, preallocate, write_at};
use crate::core::progress::{Control, JobOutcome, LiveProgress};
use crate::core::urlx::{filename_from_url, Protocol};
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use suppaftp::types::FileType;
use suppaftp::{FtpError, FtpStream};
use url::Url;

pub struct FtpTarget {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub path: String,
}

pub fn parse_ftp(url: &str) -> Result<FtpTarget, String> {
    let u = Url::parse(url).map_err(|e| e.to_string())?;
    let host = u.host_str().ok_or("FTP 地址里没有主机")?.to_string();
    let port = u.port().unwrap_or(21);
    let user = if u.username().is_empty() {
        "anonymous".into()
    } else {
        percent_encoding::percent_decode_str(u.username())
            .decode_utf8_lossy()
            .into_owned()
    };
    let pass = u
        .password()
        .map(|p| {
            percent_encoding::percent_decode_str(p)
                .decode_utf8_lossy()
                .into_owned()
        })
        .unwrap_or_else(|| "quickget@localhost".into());
    let path = u.path();
    let path = if path.is_empty() { "/" } else { path }.to_string();
    Ok(FtpTarget {
        host,
        port,
        user,
        pass,
        path,
    })
}

pub fn probe_ftp(url: &str) -> Result<crate::core::probe::Probe, String> {
    let t = parse_ftp(url)?;
    let mut ftp = connect(&t)?;
    let size = ftp.size(&t.path).unwrap_or(0) as u64;
    let _ = ftp.quit();
    Ok(crate::core::probe::Probe {
        url: url.to_string(),
        protocol: Protocol::Ftp,
        size,
        ranges: size > 0,
        filename: filename_from_url(url),
        content_type: String::new(),
        etag: None,
    })
}

fn connect(t: &FtpTarget) -> Result<FtpStream, String> {
    let addr = format!("{}:{}", t.host, t.port);
    let mut ftp = FtpStream::connect(&addr).map_err(|e| e.to_string())?;
    ftp.login(&t.user, &t.pass).map_err(|e| e.to_string())?;
    ftp.transfer_type(FileType::Binary)
        .map_err(|e| e.to_string())?;
    Ok(ftp)
}

pub struct FtpJob<'a> {
    pub url: &'a str,
    pub dest: &'a Path,
    pub part: &'a Path,
    pub progress: Arc<LiveProgress>,
    pub ctrl: Control,
}

pub fn download_ftp(job: FtpJob<'_>) -> JobOutcome {
    let t = match parse_ftp(job.url) {
        Ok(t) => t,
        Err(e) => return JobOutcome::Failed(e),
    };
    let mut ftp = match connect(&t) {
        Ok(f) => f,
        Err(e) => return JobOutcome::Failed(e),
    };
    let size = ftp.size(&t.path).unwrap_or(0) as u64;
    job.progress.set_total(size);

    let resume_from = if job.part.exists() {
        std::fs::metadata(job.part).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    if resume_from > 0 {
        job.progress
            .downloaded
            .store(resume_from, Ordering::Relaxed);
        job.progress
            .last_bytes
            .store(resume_from, Ordering::Relaxed);
        if ftp.resume_transfer(resume_from as usize).is_err() {
            let _ = std::fs::remove_file(job.part);
            job.progress.downloaded.store(0, Ordering::Relaxed);
        }
    }

    let mut file = match open_part(job.part) {
        Ok(f) => f,
        Err(e) => return JobOutcome::Failed(e.to_string()),
    };
    if size > 0 {
        preallocate(&file, size);
    }

    let cursor = job.progress.downloaded();
    let result = ftp.retr(&t.path, |reader| {
        let mut buf = vec![0u8; 256 * 1024];
        let mut offset = cursor;
        loop {
            if job.ctrl.interrupted() {
                break;
            }
            let n = reader.read(&mut buf).map_err(FtpError::ConnectionError)?;
            if n == 0 {
                break;
            }
            write_at(&mut file, offset, &buf[..n]).map_err(FtpError::ConnectionError)?;
            offset += n as u64;
            job.progress.add(n as u64);
        }
        Ok(())
    });

    let _ = ftp.quit();

    if job.ctrl.interrupted() {
        return JobOutcome::from_interrupt(&job.ctrl, job.progress.downloaded(), size);
    }
    match result {
        Ok(_) => {
            drop(file);
            if let Err(e) = finalize_part(job.part, job.dest) {
                return JobOutcome::Failed(e.to_string());
            }
            let final_size = job.progress.downloaded().max(size);
            job.progress.set_total(final_size);
            JobOutcome::Completed {
                size: final_size,
                path: job.dest.to_path_buf(),
            }
        }
        Err(e) => JobOutcome::Failed(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_anonymous_and_auth() {
        let t = parse_ftp("ftp://example.com/pub/a.zip").unwrap();
        assert_eq!(t.host, "example.com");
        assert_eq!(t.port, 21);
        assert_eq!(t.user, "anonymous");
        assert_eq!(t.path, "/pub/a.zip");

        let t = parse_ftp("ftp://bob:s3cret@h:2121/x").unwrap();
        assert_eq!(t.user, "bob");
        assert_eq!(t.pass, "s3cret");
        assert_eq!(t.port, 2121);
    }
}
