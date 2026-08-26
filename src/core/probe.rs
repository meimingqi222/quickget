//! 探测远端：体积、是否支持 Range、文件名、是否其实是 m3u8。

use crate::core::http::{build_client, probe_remote};
use crate::core::urlx::{filename_from_url, Protocol};

#[derive(Debug, Clone)]
pub struct Probe {
    pub url: String,
    pub protocol: Protocol,
    pub size: u64,
    pub ranges: bool,
    pub filename: String,
    pub content_type: String,
    pub etag: Option<String>,
}

#[derive(Debug)]
pub enum ProbeError {
    Magnet,
    Unknown,
    Network(String),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeError::Magnet => write!(f, "magnet"),
            ProbeError::Unknown => write!(f, "unknown protocol"),
            ProbeError::Network(s) => write!(f, "{s}"),
        }
    }
}

pub fn probe(url: &str, ua: &str) -> Result<Probe, ProbeError> {
    let protocol = crate::core::urlx::detect_protocol(url);
    match protocol {
        Protocol::Magnet => Ok(probe_magnet(url)),
        Protocol::Unknown => Err(ProbeError::Unknown),
        Protocol::Ftp => crate::core::ftp::probe_ftp(url).map_err(ProbeError::Network),
        Protocol::Http | Protocol::Hls => probe_http(url, ua, protocol),
    }
}

fn probe_magnet(url: &str) -> Probe {
    Probe {
        url: url.to_string(),
        protocol: Protocol::Magnet,
        size: 0,
        ranges: true,
        filename: crate::core::urlx::filename_from_source(url),
        content_type: "application/x-bittorrent".into(),
        etag: None,
    }
}

fn probe_http(url: &str, ua: &str, mut protocol: Protocol) -> Result<Probe, ProbeError> {
    let client = build_client(ua, None).map_err(|e| ProbeError::Network(e.to_string()))?;
    let info = probe_remote(&client, url, None, None).map_err(ProbeError::Network)?;
    let ctype = info.content_type.clone();
    if ctype.contains("mpegurl") || ctype.contains("m3u8") || info.final_url.contains(".m3u8") {
        protocol = Protocol::Hls;
    }
    let filename = info
        .filename
        .clone()
        .unwrap_or_else(|| filename_from_url(&info.final_url));
    Ok(Probe {
        url: info.final_url,
        protocol,
        size: info.size,
        ranges: info.ranges,
        filename,
        content_type: ctype,
        etag: info.etag,
    })
}

pub fn parse_content_range_total(header: &str) -> Option<u64> {
    // bytes 0-0/12345  或 bytes 0-0/*
    let slash = header.rfind('/')?;
    let total = header[slash + 1..].trim();
    if total == "*" {
        return None;
    }
    total.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_content_range() {
        assert_eq!(parse_content_range_total("bytes 0-0/12345"), Some(12345));
        assert_eq!(parse_content_range_total("bytes 0-499/500"), Some(500));
        assert_eq!(parse_content_range_total("bytes 0-0/*"), None);
    }
}
