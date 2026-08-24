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
            Protocol::Magnet => "BT",
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
    if lower.starts_with("file://") {
        return Some(s.to_string());
    }
    if looks_like_torrent_path(s) {
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

fn looks_like_torrent_path(s: &str) -> bool {
    let path = s.split(['?', '#']).next().unwrap_or(s);
    let lower = path.to_ascii_lowercase().replace('\\', "/");
    if !lower.ends_with(".torrent") {
        return false;
    }
    path.contains('/')
        || path.contains('\\')
        || (cfg!(windows) && path.len() >= 3 && path.as_bytes()[1] == b':')
        || std::path::Path::new(s).is_file()
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
    if looks_like_torrent(url) {
        return Protocol::Magnet;
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Protocol::Http;
    }
    Protocol::Unknown
}

pub fn looks_like_torrent(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("magnet:") {
        return false;
    }
    if lower.starts_with("file://") {
        return looks_like_torrent_path(&lower);
    }
    let path = lower.split(['?', '#']).next().unwrap_or(&lower);
    let path = path.replace('\\', "/");
    path.ends_with(".torrent")
}

/// 从 URL 路径抽出文件名，百分号解码，清洗非法字符。
pub fn filename_from_url(url: &str) -> String {
    let parsed = url::Url::parse(url).ok();
    let path = parsed
        .as_ref()
        .map(|u| u.path().to_string())
        .unwrap_or_else(|| url.to_string());
    let normalized = path.replace('\\', "/");
    let last = normalized
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("download");
    let decoded = percent_decode_str(last).decode_utf8_lossy();
    let clean = sanitize_filename(&decoded);
    if clean.is_empty() || clean == "download" {
        "download.bin".into()
    } else {
        clean
    }
}

/// 磁力链接还没拿到元数据时的标题：有 `dn` 就用，否则先叫 BT，等种子信息到了再换真名。
pub fn filename_from_magnet(url: &str) -> String {
    if let Some(dn) = magnet_display_name(url) {
        return dn;
    }
    "BT".into()
}

/// 占位标题：还没换成种子真名。
pub fn is_bt_placeholder(name: &str) -> bool {
    let t = name.trim();
    if t.eq_ignore_ascii_case("bt") || t.eq_ignore_ascii_case("torrent") {
        return true;
    }
    let lower = t.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("magnet-") {
        return !rest.is_empty()
            && rest
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == ' ');
    }
    false
}

fn magnet_display_name(url: &str) -> Option<String> {
    if let Ok(u) = url::Url::parse(url) {
        for (k, v) in u.query_pairs() {
            if k.eq_ignore_ascii_case("dn") {
                let s = sanitize_filename(v.as_ref());
                if !s.is_empty() && !is_bt_placeholder(&s) {
                    return Some(s);
                }
            }
        }
    }
    if let Some(dn) = magnet_query(url, "dn") {
        let plus = dn.replace('+', " ");
        let decoded = percent_decode_str(&plus).decode_utf8_lossy();
        let s = sanitize_filename(&decoded);
        if !s.is_empty() && !is_bt_placeholder(&s) {
            return Some(s);
        }
    }
    None
}

/// 按协议挑一个还没拿到远端元数据时能用的文件名。
pub fn filename_from_source(url: &str) -> String {
    if url.to_ascii_lowercase().starts_with("magnet:") {
        return filename_from_magnet(url);
    }
    if looks_like_torrent(url) {
        let name = filename_from_url(url);
        let stripped = name
            .strip_suffix(".torrent")
            .or_else(|| name.strip_suffix(".TORRENT"))
            .unwrap_or(&name);
        let s = sanitize_filename(stripped);
        if s.is_empty() {
            return "torrent".into();
        }
        return s;
    }
    filename_from_url(url)
}

fn magnet_query(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    for part in q.split('&') {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        if k.eq_ignore_ascii_case(key) && !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
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
/// 磁力链接用 infohash（`xt`）去重。
pub fn url_identity(url: &str) -> String {
    if url.to_ascii_lowercase().starts_with("magnet:") {
        if let Some(xt) = magnet_query(url, "xt") {
            return format!("magnet:{}", xt.to_ascii_lowercase());
        }
        return url.to_ascii_lowercase();
    }
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
            let ext = &filename[i + 1..];
            if is_file_extension(ext) {
                (filename[..i].to_string(), ext.to_string())
            } else {
                (filename.to_string(), String::new())
            }
        }
        _ => (filename.to_string(), String::new()),
    }
}

/// 纯数字后缀当成版本号（`Ubuntu 24.04`），不当成扩展名。
fn is_file_extension(ext: &str) -> bool {
    let n = ext.chars().count();
    if n == 0 || n > 8 {
        return false;
    }
    let mut has_letter = false;
    for c in ext.chars() {
        if c.is_ascii_alphabetic() {
            has_letter = true;
        } else if !c.is_ascii_digit() {
            return false;
        }
    }
    has_letter
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
        assert_eq!(
            detect_protocol("https://ex.com/a.torrent"),
            Protocol::Magnet
        );
        assert_eq!(
            detect_protocol("https://ex.com/a.torrent?token=1"),
            Protocol::Magnet
        );
        assert_eq!(
            detect_protocol(r"C:\Downloads\ubuntu.torrent"),
            Protocol::Magnet
        );
    }

    #[test]
    fn filename_from_magnet_uses_dn() {
        assert_eq!(
            filename_from_magnet("magnet:?xt=urn:btih:abcdef0123456789&dn=Ubuntu%2024.04"),
            "Ubuntu 24.04"
        );
        assert_eq!(
            filename_from_magnet("magnet:?xt=urn:btih:abcdef0123456789&dn=Foo+Bar"),
            "Foo Bar"
        );
        assert_eq!(
            filename_from_magnet("magnet:?xt=urn:btih:abcdef0123456789"),
            "BT"
        );
        assert!(is_bt_placeholder("BT"));
        assert!(is_bt_placeholder("magnet-abcdef01"));
        assert!(!is_bt_placeholder("Ubuntu 24.04"));
    }

    #[test]
    fn magnet_identity_is_infohash() {
        let a = "magnet:?xt=urn:btih:ABCDEF&dn=one&tr=http://t";
        let b = "magnet:?xt=urn:btih:abcdef&dn=two";
        assert_eq!(url_identity(a), url_identity(b));
        assert_eq!(url_identity(a), "magnet:urn:btih:abcdef");
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
    fn unique_path_keeps_version_dot_in_folder_name() {
        let dir = std::env::temp_dir().join(format!("qg-uniq-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("Ubuntu 24.04")).unwrap();
        let (name, _) = unique_path(&dir, "Ubuntu 24.04");
        assert_eq!(name, "Ubuntu 24.04 (1)");
        let (zip, _) = unique_path(&dir, "a.zip");
        assert_eq!(zip, "a.zip");
        std::fs::write(dir.join("a.zip"), b"x").unwrap();
        let (zip2, _) = unique_path(&dir, "a.zip");
        assert_eq!(zip2, "a (1).zip");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_multiple_lines() {
        let raw = "https://a.com/1\nftp://b.com/2\nmagnet:?xt=urn:btih:abc\nnot a url\n";
        let v = extract_urls(raw);
        assert_eq!(v.len(), 3);
        assert!(v[2].starts_with("magnet:"));
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
