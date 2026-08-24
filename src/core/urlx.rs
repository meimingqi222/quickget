//! URL 识别、协议判定、文件名清洗。

use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    Http,
    Ftp,
    Hls,
    Magnet,
    Unknown,
}

impl Protocol {
    pub fn label(self) -> &'static str {
        match self {
            Protocol::Http => "HTTP",
            Protocol::Ftp => "FTP",
            Protocol::Hls => "HLS",
            Protocol::Magnet => "Magnet",
            Protocol::Unknown => "?",
        }
    }
}

/// 从用户粘贴的原始文本抽出一条或多条链接。
pub fn extract_urls(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(u) = normalize_url(t) {
            out.push(u);
        }
    }
    if out.is_empty() {
        if let Some(u) = normalize_url(raw.trim()) {
            out.push(u);
        }
    }
    out
}

/// 补全协议、去掉包裹引号与空白。磁力链接原样返回。
pub fn normalize_url(raw: &str) -> Option<String> {
    let s = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '<' || c == '>' || c == '`');
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("magnet:") {
        return Some(s.to_string());
    }
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("ftp://")
        || lower.starts_with("ftps://")
    {
        return Some(s.to_string());
    }
    // 看起来像域名或 IP 的，默认补 https。
    if looks_like_host_path(s) {
        return Some(format!("https://{s}"));
    }
    None
}

fn looks_like_host_path(s: &str) -> bool {
    let host = s.split('/').next().unwrap_or(s);
    if host.contains(' ') || host.contains('\t') {
        return false;
    }
    host.contains('.') || host.contains(':')
}

pub fn detect_protocol(url: &str) -> Protocol {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("magnet:") {
        return Protocol::Magnet;
    }
    if lower.starts_with("ftp://") || lower.starts_with("ftps://") {
        return Protocol::Ftp;
    }
    if lower.contains(".m3u8") || lower.contains("m3u8?") || lower.contains("/m3u8") {
        return Protocol::Hls;
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Protocol::Http;
    }
    Protocol::Unknown
}

/// 从 URL 路径抽出文件名，百分号解码，清洗非法字符。
pub fn filename_from_url(url: &str) -> String {
    let parsed = url::Url::parse(url).ok();
    let path = parsed
        .as_ref()
        .map(|u| u.path().to_string())
        .unwrap_or_else(|| url.to_string());
    let last = path.rsplit('/').find(|s| !s.is_empty()).unwrap_or("download");
    let decoded = percent_decode_str(last).decode_utf8_lossy();
    let clean = sanitize_filename(&decoded);
    if clean.is_empty() || clean == "download" {
        "download.bin".into()
    } else {
        clean
    }
}

/// Content-Disposition: attachment; filename="x"; filename*=UTF-8''x
pub fn filename_from_disposition(header: &str) -> Option<String> {
    // filename* 优先（RFC 5987）
    if let Some(star) = find_param(header, "filename*") {
        let v = star.trim_matches('"');
        let decoded = if let Some(rest) = v.strip_prefix("UTF-8''").or_else(|| v.strip_prefix("utf-8''"))
        {
            percent_decode_str(rest).decode_utf8_lossy().into_owned()
        } else {
            v.to_string()
        };
        let s = sanitize_filename(&decoded);
        if !s.is_empty() {
            return Some(s);
        }
    }
    if let Some(plain) = find_param(header, "filename") {
        let v = plain.trim_matches('"').trim_matches('\'');
        let s = sanitize_filename(v);
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}

fn find_param<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    let lower = header.to_ascii_lowercase();
    let key = format!("{name}=");
    let idx = lower.find(&key)?;
    let rest = &header[idx + key.len()..];
    let end = rest.find(';').unwrap_or(rest.len());
    Some(rest[..end].trim())
}

pub fn sanitize_filename(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' | '\n' | '\r' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    s = s.trim().trim_matches('.').to_string();
    if s.chars().count() > 180 {
        s = s.chars().take(180).collect();
    }
    s
}

/// 去掉 query / fragment，用来判断是不是同一个资源。
/// 地理空间数据云的 sid 每次都会变，比完整 URL 才能去重。
pub fn url_identity(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => format!(
            "{}://{}{}",
            u.scheme(),
            u.host_str().unwrap_or(""),
            u.path()
        ),
        Err(_) => url.split(['?', '#']).next().unwrap_or(url).to_string(),
    }
}

/// 目标目录里已有同名文件时，变成 `name (1).ext`。
pub fn unique_path(dir: &std::path::Path, filename: &str) -> (String, std::path::PathBuf) {
    let dest = dir.join(filename);
    if !dest.exists() && !part_exists(dir, filename) {
        return (filename.to_string(), dest);
    }
    let (stem, ext) = split_name(filename);
    for i in 1..10_000 {
        let candidate = if ext.is_empty() {
            format!("{stem} ({i})")
        } else {
            format!("{stem} ({i}).{ext}")
        };
        if !dir.join(&candidate).exists() && !part_exists(dir, &candidate) {
            let p = dir.join(&candidate);
            return (candidate, p);
        }
    }
    (filename.to_string(), dest)
}

fn part_exists(dir: &std::path::Path, filename: &str) -> bool {
    let mut p = dir.join(filename).into_os_string();
    p.push(".part");
    std::path::PathBuf::from(p).exists()
}

fn split_name(filename: &str) -> (String, String) {
    match filename.rfind('.') {
        Some(i) if i > 0 && i < filename.len() - 1 => {
            (filename[..i].to_string(), filename[i + 1..].to_string())
        }
        _ => (filename.to_string(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_http_and_bare_host() {
        assert_eq!(
            normalize_url("https://example.com/a.zip").as_deref(),
            Some("https://example.com/a.zip")
        );
        assert_eq!(
            normalize_url("example.com/a.zip").as_deref(),
            Some("https://example.com/a.zip")
        );
        assert_eq!(
            normalize_url("  \"ftp://h/x\"  ").as_deref(),
            Some("ftp://h/x")
        );
        assert!(normalize_url("just some words").is_none());
    }

    #[test]
    fn detect_protocols() {
        assert_eq!(
            detect_protocol("https://x.com/a.bin"),
            Protocol::Http
        );
        assert_eq!(detect_protocol("ftp://h/a"), Protocol::Ftp);
        assert_eq!(
            detect_protocol("https://cdn.example/play.m3u8"),
            Protocol::Hls
        );
        assert_eq!(
            detect_protocol("magnet:?xt=urn:btih:abc"),
            Protocol::Magnet
        );
    }

    #[test]
    fn filename_from_url_decodes() {
        assert_eq!(
            filename_from_url("https://ex.com/files/%E4%B8%AD%E6%96%87.zip"),
            "中文.zip"
        );
        assert_eq!(
            filename_from_url("https://ex.com/a/b/c.tar.gz"),
            "c.tar.gz"
        );
    }

    #[test]
    fn disposition_rfc5987() {
        let h = r#"attachment; filename="fallback.bin"; filename*=UTF-8''%E6%B5%8B%E8%AF%95.bin"#;
        assert_eq!(filename_from_disposition(h).as_deref(), Some("测试.bin"));
    }

    #[test]
    fn extract_multiple_lines() {
        let raw = "https://a.com/1\nftp://b.com/2\nnot a url\n";
        let v = extract_urls(raw);
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn url_identity_strips_query() {
        assert_eq!(
            url_identity("https://bjdl.gscloud.cn/sources/download/447/a.SAFE?sid=abc&uid=1"),
            "https://bjdl.gscloud.cn/sources/download/447/a.SAFE"
        );
        assert_eq!(
            url_identity("https://bjdl.gscloud.cn/sources/download/447/a.SAFE?sid=zzz"),
            url_identity("https://bjdl.gscloud.cn/sources/download/447/a.SAFE?sid=yyy&uid=2")
        );
    }
}
