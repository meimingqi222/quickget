//! 百度网盘分享解析（ShareProvider 插件）。
//!
//! 协议链路：
//!   1. `GET /share/tplconfig?surl=<完整id>&fields=sign,timestamp&view_mode=1`
//!        → 新鲜的 sign + timestamp（无需登录态）；
//!   2. `GET /share/list?shorturl=<去掉首字符>&root=1&page=1&num=100&…`
//!        → share_id、uk、文件列表（fs_id/server_filename/size/isdir）；
//!          注意百度的不一致：tplconfig 用完整 id，share/list 的 shorturl 要去掉
//!          路径里开头的 `1`；
//!   3. `POST /api/sharedownload?…&sign=&timestamp=`，表单
//!      `encrypt=0&type=nolimit&product=share&timestamp=&uk=&primaryid=<share_id>&fid_list=[…]`
//!      （带提取码的分享再带 `extra={"sekey":<BDCLND 解码>}`）。
//!   4. 已登录时转存到网盘 `/QuickGet/.tmp/<随机id>`，再用 PCS
//!      `GET https://pcs.baidu.com/rest/2.0/pcs/file?app_id=…&method=download&path=<网盘路径>`
//!      + BDUSS Cookie + 安卓 UA → 302 到 `*.baidupcs.com/file/…?bkt=…` 真实直链。
//!   5. 真实直链交给 HTTP 引擎，用 [`DOWNLOAD_UA`]（安卓客户端 UA）下载，不带 Cookie。
//!
//! ## 逆向增强（可选，不入 Git）
//!
//! 启用 Cargo feature `baidu-reverse` 后，第 3 步改用 `encrypt=1` 请求，
//! 服务端返回加密 list，由 `baidu_reverse` 模块在本地 DES-ECB 解密后直接获取
//! dlink，无需转存。该模块包含逆向百度官方客户端获得的算法，出于版权考虑
//! 不纳入公开仓库（`baidu_reverse.rs` 已加入 `.gitignore`）。
//!
//! 隐私与加速：真实直链的 bkt 签名已含账号权益（SVIP 满速等），下载不再带
//! Cookie，避免 BDUSS 流向 CDN 域。BDUSS 只用于上面 2、3、4 步网盘/PCS 域内调用。

use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use serde_json::Value;
use std::time::Duration;

use super::{AccountInfo, NetdiskRequest, ResolvedFile, ShareProvider, Speed};

const HOST: &str = "https://pan.baidu.com";
const APP_ID: &str = "250528";
/// 直链下载用 UA：百度安卓客户端 UA。PCS `file?method=download` 302 出来的
/// 真实直链（`*.baidupcs.com/file/...?bkt=...`）只认这种 UA；`netdisk`、
/// `pan.baidu.com`、浏览器 UA 全被 CDN 以 31362 sign error 拒掉。解析阶段
/// 继续用浏览器 UA（传入参数），不在此处。
const DOWNLOAD_UA: &str = "netdisk;P2SP;3.0.0.8;netdisk;11.12.3;ANG-AN00;android-android;10.0;JSbridge4.4.0;jointBridge;1.1.0;";
/// 转存目录。网页 sharedownload 现在常回密文，登录态改走这条换链。
const SAVE_DIR: &str = "/QuickGet";
/// 解析只带这些 Cookie：完整页面 Cookie 里夹着统计字段，偶尔会让头非法而被整段丢弃。
const COOKIE_KEEP: &[&str] = &[
    "BDUSS",
    "BDUSS_BFESS",
    "STOKEN",
    "BAIDUID",
    "BAIDUID_BFESS",
    "BDCLND",
    "PANWEB",
    "ndut_fmt",
];

/// application/x-www-form-urlencoded：除 RFC 3986 非保留字符外全部转义。
const FORM_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

pub struct Baidu;

impl ShareProvider for Baidu {
    fn id(&self) -> &'static str {
        "baidu"
    }

    fn display_name(&self) -> &'static str {
        "百度网盘"
    }

    fn matches(&self, url: &str) -> bool {
        is_baidu_share_url(url)
    }

    fn check_login(&self, cookies: &str, ua: &str) -> Result<AccountInfo, String> {
        check_login(cookies, ua)
    }

    fn resolve(
        &self,
        req: &NetdiskRequest,
        cookies: &str,
        ua: &str,
    ) -> Result<Vec<ResolvedFile>, String> {
        resolve_share(req, cookies, ua)
    }

    fn download_ua(&self) -> Option<&'static str> {
        Some(DOWNLOAD_UA)
    }

    fn max_part_size(&self) -> Option<u64> {
        // PCS 直链对单次 Range > 4MB 回 31326 hitcode:104 风控。
        // 实测 4MB 安全、8MB 触发，取 4MB 留余量。
        Some(4 * 1024 * 1024)
    }
}

pub fn enc(s: &str) -> String {
    utf8_percent_encode(s, FORM_SET).to_string()
}

/// 只保留解析需要的 Cookie，避免整段头因个别非法值被丢弃。
pub fn select_cookies(raw: &str) -> String {
    raw.split(';')
        .filter_map(|pair| {
            let pair = pair.trim();
            if pair.is_empty() {
                return None;
            }
            let name = pair.split_once('=').map(|(n, _)| n.trim()).unwrap_or(pair);
            COOKIE_KEEP
                .iter()
                .any(|keep| keep.eq_ignore_ascii_case(name))
                .then_some(pair)
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn attach_cookies(
    mut req: reqwest::blocking::RequestBuilder,
    cookies: &str,
) -> reqwest::blocking::RequestBuilder {
    let cookies = cookies.trim();
    if cookies.is_empty() {
        return req;
    }
    match reqwest::header::HeaderValue::from_str(cookies) {
        Ok(v) => req = req.header(reqwest::header::COOKIE, v),
        Err(e) => crate::core::log::write(format_args!("百度 Cookie 头无效，已丢弃：{e}")),
    }
    req
}

/// 是否是我们能处理的分享链接。
pub fn is_baidu_share_url(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else {
        return false;
    };
    if u.host_str() != Some("pan.baidu.com") {
        return false;
    }
    let path = u.path();
    path.starts_with("/s/") || path.starts_with("/share/")
}

/// 从分享链接提取 id。返回 (tplconfig 用的完整 id, share/list 用的 shorturl)。
///
/// 百度约定分享 id 以 `1` 开头，share/list 的 shorturl 参数要去掉这第一个字符：
/// `/s/1AbcDefGhiJklMnoPqrStu` → tpl `1AbcDefGhiJklMnoPqrStu`，
/// shorturl `AbcDefGhiJklMnoPqrStu`（去掉开头的 `1`）。id 不是 1 开头时两者相同。
pub fn parse_surls(url: &str) -> Result<(String, String), String> {
    let u = url::Url::parse(url.trim()).map_err(|_| format!("无法解析链接：{url}"))?;
    let id: Option<String> = if let Some(segs) = u.path_segments() {
        let seg: Vec<String> = segs.filter(|s| !s.is_empty()).map(str::to_string).collect();
        match seg.as_slice() {
            [tag, id] if tag == "s" => Some(id.clone()),
            [second, third] if second == "share" => {
                // /share/init?surl=xxx 或 /share/xxxx
                if third == "init" {
                    match u.query_pairs().find(|(k, _)| k == "surl") {
                        Some((_, v)) => Some(v.to_string()),
                        None => None,
                    }
                } else {
                    Some(third.clone())
                }
            }
            _ => None,
        }
    } else {
        None
    };
    let Some(id) = id else {
        return Err(format!("链接里找不到分享 id：{url}"));
    };
    if id.len() < 6 || !id.chars().all(|c: char| c.is_ascii_alphanumeric()) {
        return Err(format!("分享 id 形态异常：{id}"));
    }
    let short: String = id.strip_prefix('1').unwrap_or(&id).to_string();
    Ok((id, short))
}

/// 从 Cookie 头里取 BDCLND 并做百分号解码，即 sharedownload 的 extra.sekey。
pub fn bdclnd_from_cookies(cookies: &str) -> Option<String> {
    for pair in cookies.split(';') {
        let pair = pair.trim();
        if let Some(v) = pair.strip_prefix("BDCLND=") {
            let decoded = percent_encoding::percent_decode_str(v)
                .decode_utf8_lossy()
                .to_string();
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }
    None
}

fn get_json(
    client: &reqwest::blocking::Client,
    url: String,
    referer: &str,
    cookies: Option<&str>,
) -> Result<Value, String> {
    let mut req = client
        .get(url)
        .header("Referer", referer)
        .header("X-Requested-With", "XMLHttpRequest")
        .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
        .timeout(Duration::from_secs(20));
    if let Some(c) = cookies.map(str::trim).filter(|c| !c.is_empty()) {
        req = attach_cookies(req, c);
    }
    let resp = req.send().map_err(|e| format!("请求失败：{e}"))?;
    let status = resp.status();
    let text = resp.text().map_err(|e| format!("读取响应失败：{e}"))?;
    if !status.is_success() {
        return Err(format!("百度网盘返回 HTTP {status}"));
    }
    serde_json::from_str(text.trim()).map_err(|_| "百度网盘返回了非 JSON 响应".to_string())
}

fn fetch_html(
    client: &reqwest::blocking::Client,
    url: &str,
    cookies: &str,
) -> Option<String> {
    let mut req = client
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml")
        .timeout(Duration::from_secs(20));
    req = attach_cookies(req, cookies);
    let resp = req.send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text().ok()
}

fn post_form(
    client: &reqwest::blocking::Client,
    url: String,
    referer: &str,
    cookies: &str,
    body: String,
) -> Result<(u16, String), String> {
    let mut http = client
        .post(url)
        .header("Referer", referer)
        .header("Origin", HOST)
        .header("X-Requested-With", "XMLHttpRequest")
        .timeout(Duration::from_secs(30))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body);
    http = attach_cookies(http, cookies);
    let resp = http.send().map_err(|e| format!("请求失败：{e}"))?;
    let status = resp.status().as_u16();
    let text = resp.text().map_err(|e| format!("读取响应失败：{e}"))?;
    Ok((status, text))
}

fn post_json(
    client: &reqwest::blocking::Client,
    url: String,
    referer: &str,
    cookies: &str,
    body: String,
) -> Result<Value, String> {
    let (status, text) = post_form(client, url, referer, cookies, body)?;
    if !(200..300).contains(&status) {
        return Err(format!("百度网盘返回 HTTP {status}"));
    }
    serde_json::from_str(text.trim()).map_err(|_| {
        format!(
            "百度网盘返回了非 JSON 响应：{}",
            truncate_head(&text, 200)
        )
    })
}

#[cfg(test)]
fn response_is_encrypted(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.get("list").cloned())
        .is_some_and(|l| matches!(l, Value::String(s) if !s.is_empty()))
}

pub fn errno_of(v: &Value) -> Result<i64, String> {
    v.get("errno")
        .and_then(|e| e.as_i64())
        .ok_or_else(|| "百度网盘响应缺少 errno（接口可能改版）".to_string())
}

/// 取 bdstoken（CSRF）。失败不影响主流程，返回 None。
fn fetch_bdstoken(client: &reqwest::blocking::Client, referer: &str, cookies: &str) -> Option<String> {
    let url = format!(
        "{HOST}/api/gettemplatevariable?clienttype=0&web=1&app_id={APP_ID}&fields=%5B%22bdstoken%22%5D"
    );
    let v = get_json(client, url, referer, Some(cookies)).ok()?;
    v.pointer("/data/bdstoken")
        .or_else(|| v.pointer("/result/bdstoken"))
        .and_then(|b| b.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 第 1 步：新鲜的 sign + timestamp。
pub fn parse_tplconfig(v: &Value) -> Result<(String, String), String> {
    let errno = errno_of(v)?;
    if errno != 0 {
        return Err(errno_message(errno));
    }
    let data = v.get("data").ok_or_else(|| "tplconfig 缺少 data 字段".to_string())?;
    let ts =
        json_as_string(data.get("timestamp")).ok_or_else(|| "tplconfig 缺少 timestamp".to_string())?;
    let sign = json_as_string(data.get("sign")).ok_or_else(|| "tplconfig 缺少 sign".to_string())?;
    if ts.is_empty() || sign.is_empty() {
        return Err("tplconfig 返回的 sign/timestamp 为空".into());
    }
    Ok((sign, ts))
}

/// 第 2 步产物：分享元信息 + 顶层文件列表。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareInfo {
    pub share_id: String,
    pub uk: String,
    pub title: Option<String>,
    pub files: Vec<ShareFile>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShareFile {
    pub fs_id: String,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

/// 百度接口里数字/字符串随机出现，统一转字符串。
pub fn json_as_string(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

pub fn parse_share_list(v: &Value) -> Result<ShareInfo, String> {
    let errno = errno_of(v)?;
    if errno != 0 {
        return Err(errno_message(errno));
    }
    let share_id =
        json_as_string(v.get("share_id")).ok_or_else(|| "share/list 缺少 share_id".to_string())?;
    let uk = json_as_string(v.get("uk")).ok_or_else(|| "share/list 缺少 uk".to_string())?;
    let title = v
        .get("title")
        .and_then(|t| t.as_str())
        .map(|t| t.trim_matches('/').to_string())
        .filter(|t| !t.is_empty());
    let mut files = Vec::new();
    if let Some(list) = v.get("list").and_then(|l| l.as_array()) {
        for item in list {
            let Some(fs_id) = json_as_string(item.get("fs_id")) else {
                continue;
            };
            let is_dir = item
                .get("isdir")
                .and_then(|d| {
                    d.as_str()
                        .map(str::to_string)
                        .or_else(|| d.as_i64().map(|n| n.to_string()))
                })
                .map(|d| d == "1")
                .unwrap_or(false);
            let name = item
                .get("server_filename")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_string();
            let size = item
                .get("size")
                .and_then(|s| json_as_string(Some(s)))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            files.push(ShareFile {
                fs_id,
                name,
                size,
                is_dir,
            });
        }
    }
    Ok(ShareInfo {
        share_id,
        uk,
        title,
        files,
    })
}

/// 第 3 步：sharedownload 的查询串。sign/timestamp 必须成对，来自 tplconfig。
pub fn request_query(sign: &str, timestamp: &str) -> String {
    request_query_full(sign, timestamp, None, None, None)
}

fn request_query_full(
    sign: &str,
    timestamp: &str,
    bdstoken: Option<&str>,
    logid: Option<&str>,
    js_token: Option<&str>,
) -> String {
    let mut q = format!(
        "app_id={APP_ID}&channel=chunlei&clienttype=0&web=1&sign={}&timestamp={}",
        enc(sign),
        enc(timestamp),
    );
    if let Some(v) = bdstoken.map(str::trim).filter(|s| !s.is_empty()) {
        q.push_str("&bdstoken=");
        q.push_str(&enc(v));
    }
    if let Some(v) = logid.map(str::trim).filter(|s| !s.is_empty()) {
        q.push_str("&logid=");
        q.push_str(&enc(v));
    }
    if let Some(v) = js_token.map(str::trim).filter(|s| !s.is_empty()) {
        q.push_str("&jsToken=");
        q.push_str(&enc(v));
    }
    q
}

/// 页面 boot 脚本把 BAIDUID 做标准 Base64 当作 logid。
pub fn logid_from_cookies(cookies: &str) -> Option<String> {
    cookie_named(cookies, "BAIDUID").map(|v| b64_std(v.as_bytes()))
}

fn cookie_named(cookies: &str, name: &str) -> Option<String> {
    for pair in cookies.split(';') {
        let pair = pair.trim();
        if let Some(v) = pair
            .split_once('=')
            .filter(|(n, _)| n.trim().eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim())
        {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// 从分享页 HTML（或 URL 编码后的内联脚本）抠 jsToken。
pub fn extract_js_token(html: &str) -> Option<String> {
    for marker in [r#"fn(""#, "fn(%22", "fn%28%22"] {
        if let Some(i) = html.find(marker) {
            let rest = &html[i + marker.len()..];
            let hex: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
            if hex.len() >= 32 {
                return Some(hex);
            }
        }
    }
    None
}

fn b64_std(input: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i < input.len() {
        let remain = input.len() - i;
        let b0 = input[i];
        let b1 = if remain > 1 { input[i + 1] } else { 0 };
        let b2 = if remain > 2 { input[i + 2] } else { 0 };
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if remain > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if remain > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
        i += 3;
    }
    out
}

pub fn form_body(
    timestamp: &str,
    uk: &str,
    share_id: &str,
    fids: &[String],
    sekey: Option<&str>,
) -> Result<String, String> {
    let mut ids = Vec::with_capacity(fids.len());
    for raw in fids {
        let id: u64 = raw
            .trim()
            .parse()
            .map_err(|_| format!("无效的文件 id：{raw}"))?;
        ids.push(id.to_string());
    }
    if ids.is_empty() {
        return Err("没有可下载的文件".into());
    }
    // encrypt=0 请求明文 list。启用 baidu-reverse feature 时由
    // baidu_reverse::form_body_encrypted 改用 encrypt=1 + DES 解密。
    let mut body = format!(
        "encrypt=0&type=nolimit&product=share&timestamp={}&uk={}&primaryid={}&fid_list=%5B{}%5D",
        enc(timestamp),
        enc(uk),
        enc(share_id),
        ids.join(","),
    );
    if let Some(sekey) = sekey.map(str::trim).filter(|s| !s.is_empty()) {
        let extra = serde_json::json!({ "sekey": sekey }).to_string();
        body.push_str("&extra=");
        body.push_str(&enc(&extra));
    }
    Ok(body)
}

/// 第 3 步响应：明文 dlink 列表。
/// 启用 baidu-reverse feature 时，加密 list 由 baidu_reverse 模块处理。
pub fn parse_response(text: &str) -> Result<Vec<ResolvedFile>, String> {
    let v: Value =
        serde_json::from_str(text.trim()).map_err(|_| "百度网盘返回了非 JSON 响应".to_string())?;
    let errno = errno_of(&v)?;
    if errno != 0 {
        crate::core::log::write(format_args!(
            "百度 sharedownload errno={errno}：{}",
            truncate_head(text, 400)
        ));
        return Err(errno_message(errno));
    }
    // 服务端回密文：需要 baidu-reverse feature 才能解密。
    if let Some(Value::String(s)) = v.get("list") {
        if !s.is_empty() {
            #[cfg(feature = "baidu-reverse")]
            {
                return super::baidu_reverse::parse_response_with_decrypt(text);
            }
            #[cfg(not(feature = "baidu-reverse"))]
            {
                crate::core::log::write(format_args!(
                    "百度 sharedownload 返回加密 list（长度 {}），需要 baidu-reverse feature",
                    s.len()
                ));
                return Err(
                    "服务端返回了加密直链列表。请启用 baidu-reverse feature 后重试；\
                     若持续出现，请在浏览器里打开该分享页确认可正常下载"
                        .into(),
                );
            }
        }
    }
    // 明文 list（encrypt=0 或服务端没加密时）
    let Some(list) = v.get("list").and_then(|l| l.as_array()).cloned() else {
        crate::core::log::write(format_args!(
            "百度 sharedownload 缺少 list 字段：{}",
            truncate_head(text, 400)
        ));
        return Err("百度网盘响应缺少 list 字段（接口可能改版）".into());
    };
    parse_dlink_items(&list)
}

/// 从 dlink item 数组提取 ResolvedFile 列表。
fn parse_dlink_items(list: &[Value]) -> Result<Vec<ResolvedFile>, String> {
    let mut out = Vec::with_capacity(list.len());
    for item in list {
        let Some(dlink) = item.get("dlink").and_then(|d| d.as_str()) else {
            continue;
        };
        if dlink.is_empty() {
            continue;
        }
        // 个别入口会把 & 转义成 HTML 实体，顺手修掉。
        let dlink = dlink.replace("&amp;", "&");
        let filename = item
            .get("server_filename")
            .and_then(|n| n.as_str())
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| fallback_name(&dlink));
        let size = item
            .get("size")
            .and_then(|s| json_as_string(Some(s)))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        out.push(ResolvedFile {
            dlink,
            filename,
            size,
            transfer_path: None,
        });
    }
    if out.is_empty() {
        return Err("sharedownload 响应中没有可下载的文件".into());
    }
    Ok(out)
}

pub fn truncate_head(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect::<String>() + "…"
    }
}

pub fn fallback_name(dlink: &str) -> String {
    let path = dlink.split(['?', '#']).next().unwrap_or(dlink);
    let last = path.rsplit('/').next().unwrap_or(path);
    percent_encoding::percent_decode_str(last)
        .decode_utf8_lossy()
        .to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Transferred {
    from_fs_id: String,
    to_fs_id: Option<String>,
    to_path: Option<String>,
}

/// 转存响应：`extra.list` / `info` / `list` 三种形态都见过。
fn parse_transfer_dest(v: &Value) -> Result<Vec<Transferred>, String> {
    let errno = errno_of(v)?;
    if errno != 0 {
        return Err(errno_message(errno));
    }
    let arrays = [
        v.pointer("/extra/list"),
        v.get("info"),
        v.get("list"),
    ];
    let mut out = Vec::new();
    for arr in arrays.into_iter().flatten().filter_map(|x| x.as_array()) {
        for item in arr {
            // info[] 里常见的是分享侧 fsid/path，不能拿去换自己网盘的直链。
            let to_fs = json_as_string(item.get("to_fs_id"));
            let to_path = item
                .get("to")
                .and_then(|p| p.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());
            if to_fs.is_none() && to_path.is_none() {
                continue;
            }
            let from = json_as_string(item.get("from_fs_id")).unwrap_or_default();
            out.push(Transferred {
                from_fs_id: from,
                to_fs_id: to_fs,
                to_path,
            });
        }
        if !out.is_empty() {
            break;
        }
    }
    if out.is_empty() {
        return Err("转存成功但响应里没有文件 id".into());
    }
    Ok(out)
}

/// 自己网盘的 filemetas / download 响应。
fn parse_own_dlinks(v: &Value) -> Result<Vec<ResolvedFile>, String> {
    let errno = errno_of(v)?;
    if errno != 0 {
        return Err(errno_message(errno));
    }
    let arrays = [v.get("info"), v.get("dlink"), v.get("list")];
    let mut out = Vec::new();
    for arr in arrays.into_iter().flatten().filter_map(|x| x.as_array()) {
        for item in arr {
            let Some(dlink) = item.get("dlink").and_then(|d| d.as_str()) else {
                continue;
            };
            if dlink.is_empty() {
                continue;
            }
            let dlink = dlink.replace("&amp;", "&");
            let filename = item
                .get("server_filename")
                .or_else(|| item.get("filename"))
                .and_then(|n| n.as_str())
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    item.get("path")
                        .and_then(|p| p.as_str())
                        .and_then(|p| p.rsplit('/').next())
                        .filter(|n| !n.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| fallback_name(&dlink));
            let size = item
                .get("size")
                .and_then(|s| json_as_string(Some(s)))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            out.push(ResolvedFile {
                dlink,
                filename,
                size,
                transfer_path: None,
            });
        }
        if !out.is_empty() {
            break;
        }
    }
    Ok(out)
}

fn query_with_token(base: &str, bdstoken: Option<&str>) -> String {
    match bdstoken.map(str::trim).filter(|s| !s.is_empty()) {
        Some(t) => format!("{base}&bdstoken={}", enc(t)),
        None => base.to_string(),
    }
}

fn ensure_save_dir(
    client: &reqwest::blocking::Client,
    referer: &str,
    cookies: &str,
    bdstoken: Option<&str>,
) -> Result<(), String> {
    ensure_save_dir_path(client, referer, cookies, bdstoken, SAVE_DIR)
}

/// 在网盘创建目录（已存在则视为成功）。`path` 须是绝对路径。
fn ensure_save_dir_path(
    client: &reqwest::blocking::Client,
    referer: &str,
    cookies: &str,
    bdstoken: Option<&str>,
    path: &str,
) -> Result<(), String> {
    let url = query_with_token(
        &format!(
            "{HOST}/api/create?a=commit&channel=chunlei&clienttype=0&web=1&app_id={APP_ID}"
        ),
        bdstoken,
    );
    let body = format!(
        "path={}&isdir=1&block_list=%5B%5D",
        enc(path)
    );
    let v = post_json(client, url, referer, cookies, body)?;
    let errno = errno_of(&v).unwrap_or(-1);
    // 0 成功；-8 / 31061 目录已在。
    if errno == 0 || errno == -8 || errno == 31061 {
        Ok(())
    } else {
        Err(errno_message(errno))
    }
}

/// 生成 8 字符短 id，用于转存临时子目录名。
fn short_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // 取低 32 位转 16 进制，足够唯一且短。
    format!("{:08x}", (nanos as u64) & 0xFFFFFFFF)
}

fn dlink_via_transfer(
    client: &reqwest::blocking::Client,
    referer: &str,
    cookies: &str,
    bdstoken: Option<&str>,
    uk: &str,
    share_id: &str,
    fids: &[String],
    sekey: Option<&str>,
    sizes: &std::collections::HashMap<String, u64>,
) -> Result<Vec<ResolvedFile>, String> {
    ensure_save_dir(client, referer, cookies, bdstoken)?;
    // 每次转存用唯一子目录，避免同名文件冲突，也便于下载后清理。
    let tmp_dir = format!(
        "{SAVE_DIR}/.tmp/{}",
        short_id()
    );
    ensure_save_dir_path(client, referer, cookies, bdstoken, &tmp_dir)?;
    let body = format!(
        "fsidlist=%5B{}%5D&path={}",
        fids.join(","),
        enc(&tmp_dir)
    );
    let mut last_err = "转存失败".to_string();
    let dest = 'done: {
        for async_flag in ["0", "1"] {
            let mut q = format!(
                "shareid={}&from={}&ondup=newcopy&async={async_flag}&channel=chunlei&clienttype=0&web=1&app_id={APP_ID}",
                enc(share_id),
                enc(uk)
            );
            if let Some(t) = bdstoken.map(str::trim).filter(|s| !s.is_empty()) {
                q.push_str("&bdstoken=");
                q.push_str(&enc(t));
            }
            if let Some(s) = sekey.map(str::trim).filter(|s| !s.is_empty()) {
                q.push_str("&sekey=");
                q.push_str(&enc(s));
            }
            let url = format!("{HOST}/share/transfer?{q}");
            match post_json(client, url, referer, cookies, body.clone()) {
                Ok(v) => {
                    crate::core::log::write(format_args!(
                        "百度转存 async={async_flag}：{}",
                        truncate_head(&v.to_string(), 400)
                    ));
                    match parse_transfer_dest(&v) {
                        Ok(d) => break 'done d,
                        Err(e) => last_err = e,
                    }
                }
                Err(e) => last_err = e,
            }
        }
        return Err(last_err);
    };

    let paths: Vec<String> = dest.iter().filter_map(|d| d.to_path.clone()).collect();
    // 优先走 PCS download：BDUSS + 安卓 UA → 302 真实直链（*.baidupcs.com?bkt=…）。
    // 网页通道 filemetas 拿到的 dlink 下载时 CDN 回 31362 sign error（缺 access_token
    // 身份），只有 PCS file?method=download 用 BDUSS 走 302 出来的 bkt 直链能下。
    if !paths.is_empty() {
        // 用分享侧 from_fs_id 查回 size（转存响应本身不带 size）。
        let items: Vec<(String, u64)> = dest
            .iter()
            .filter_map(|d| d.to_path.as_ref().map(|p| {
                let sz = sizes.get(&d.from_fs_id).copied().unwrap_or(0);
                (p.clone(), sz)
            }))
            .collect();
        match pcs_download_link(cookies, &items) {
            Ok(files) if !files.is_empty() => return Ok(files),
            Ok(_) => crate::core::log::write(format_args!(
                "百度 PCS download 返回空，回退 filemetas"
            )),
            Err(e) => crate::core::log::write(format_args!(
                "百度 PCS download 失败：{e}，回退 filemetas"
            )),
        }
    }

    // 回退：网页通道 filemetas 换 dlink（可能 sign error，但保留作兜底）。
    let fsids: Vec<String> = dest.iter().filter_map(|d| d.to_fs_id.clone()).collect();
    let meta_url = query_with_token(
        &format!(
            "{HOST}/api/filemetas?dlink=1&channel=chunlei&clienttype=0&web=1&app_id={APP_ID}"
        ),
        bdstoken,
    );
    let meta_body = if !fsids.is_empty() {
        format!("fsids=%5B{}%5D", fsids.join(","))
    } else if !paths.is_empty() {
        format!("target={}", enc(&serde_json::json!(paths).to_string()))
    } else {
        return Err("转存响应缺少目标文件".into());
    };
    let meta = post_json(client, meta_url, referer, cookies, meta_body)?;
    crate::core::log::write(format_args!(
        "百度 filemetas：{}",
        truncate_head(&meta.to_string(), 400)
    ));
    let files = parse_own_dlinks(&meta)?;
    if files.is_empty() && !fsids.is_empty() {
        // filemetas 没给 dlink 时再试一次自己网盘的 download 接口。
        let dl_url = query_with_token(
            &format!("{HOST}/api/download?channel=chunlei&clienttype=0&web=1&app_id={APP_ID}"),
            bdstoken,
        );
        let dl_body = format!("fidlist=%5B{}%5D&type=dlink", fsids.join(","));
        let dl = post_json(client, dl_url, referer, cookies, dl_body)?;
        return parse_own_dlinks(&dl);
    }
    if files.is_empty() {
        return Err("转存成功但没拿到直链".into());
    }
    Ok(files)
}

/// PCS `file?method=download` 换真实直链。
///
/// 用 BDUSS Cookie + 安卓客户端 UA 请求
/// `https://pcs.baidu.com/rest/2.0/pcs/file?app_id=…&method=download&path=<网盘路径>`，
/// 服务端回 302，`Location` 即 `*.baidupcs.com/file/…?bkt=…` 真实直链。
/// 该直链只需 [`DOWNLOAD_UA`] 即可下载，不带 Cookie（bkt 签名已含身份）。
///
/// 这是 BaiduPCS-Go 的下载方式，无需 OAuth access_token；网页通道 filemetas
/// 的 dlink 因缺 access_token 身份会被 CDN 以 31362 sign error 拒掉。
fn pcs_download_link(cookies: &str, items: &[(String, u64)]) -> Result<Vec<ResolvedFile>, String> {
    let client = reqwest::blocking::Client::builder()
        .use_rustls_tls()
        .http1_only()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(30))
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            if let Ok(v) = reqwest::header::HeaderValue::from_str(DOWNLOAD_UA) {
                h.insert(reqwest::header::USER_AGENT, v);
            }
            h.insert(reqwest::header::ACCEPT, reqwest::header::HeaderValue::from_static("*/*"));
            h
        })
        .build()
        .map_err(|e| format!("构建 PCS client 失败：{e}"))?;
    let mut out = Vec::with_capacity(items.len());
    for (path, size) in items {
        let url = format!(
            "https://pcs.baidu.com/rest/2.0/pcs/file?app_id={APP_ID}&method=download&path={}",
            enc(path)
        );
        let resp = client
            .get(&url)
            .header(reqwest::header::COOKIE, cookies)
            .send()
            .map_err(|e| format!("PCS download 请求失败：{e}"))?;
        let status = resp.status();
        if status.as_u16() != 302 {
            let body = resp.text().unwrap_or_default();
            return Err(format!(
                "PCS download 返回 {}：{}",
                status,
                truncate_head(&body, 200)
            ));
        }
        let Some(loc) = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
        else {
            return Err("PCS download 返回 302 但缺 Location".into());
        };
        crate::core::log::write(format_args!(
            "百度 PCS download 302：{}",
            truncate_head(&loc, 160)
        ));
        let filename = path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .map(|s| {
                percent_encoding::percent_decode_str(s)
                    .decode_utf8_lossy()
                    .to_string()
            })
            .unwrap_or_else(|| fallback_name(&loc));
        out.push(ResolvedFile {
            dlink: loc,
            filename,
            size: *size,
            transfer_path: Some(path.clone()),
        });
    }
    Ok(out)
}

/// 删除网盘里的文件/目录。用于下载完成后清理转存的临时文件。
///
/// `POST /api/filemanager?opera=delete`，body `filelist=["/path1","/path2"]`。
/// 失败只记日志不报错——清理是尽力而为，不应阻断下载流程。
pub fn delete_files(cookies: &str, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    let trimmed = select_cookies(cookies);
    let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36 Edg/151.0.0.0";
    let client = match crate::core::http::build_client(ua) {
        Ok(c) => c,
        Err(e) => {
            crate::core::log::write(format_args!("百度清理：构建 client 失败：{e}"));
            return;
        }
    };
    let bdstoken = fetch_bdstoken(&client, HOST, &trimmed);
    let url = query_with_token(
        &format!(
            "{HOST}/api/filemanager?opera=delete&async=0&onnest=fail&channel=chunlei&clienttype=0&web=1&app_id={APP_ID}"
        ),
        bdstoken.as_deref(),
    );
    let filelist = serde_json::json!(paths).to_string();
    let body = format!("filelist={}", enc(&filelist));
    match post_json(&client, url, HOST, &trimmed, body) {
        Ok(v) => {
            let errno = errno_of(&v).unwrap_or(-1);
            crate::core::log::write(format_args!(
                "百度清理 {} 个文件：errno={}",
                paths.len(),
                errno
            ));
        }
        Err(e) => crate::core::log::write(format_args!("百度清理失败：{e}")),
    }
}

/// 常见 errno 的可读解释；没收录的原样带出数字。
pub fn errno_message(errno: i64) -> String {
    match errno {
        -62 => "触发风控：请在浏览器里刷新该分享页完成验证后重试",
        -9 => "文件不存在或已被移除",
        -12 => "该分享已被取消或删除",
        -6 => "身份验证失败：Cookie 已失效，请重新登录百度账号",
        -20 => "今日下载流量已达上限（账号限速额度用完）",
        -33 => "该分享需要提取码，请先在浏览器里打开并输入提取码",
        -70 => "分享者账号异常，分享不可用",
        105 => "分享链接无效",
        2 => "参数被拒绝：sign 可能已过期，请稍后重试",
        12 => "网盘里已有同名文件",
        -7 | -30 => "转存路径无效",
        31061 => "网盘里已有同名文件",
        130 => "网盘容量不足，转存失败",
        9019 => "账号验证失败(9019 need verify)：服务端不接受当前登录态。\n\
                请确认浏览器已登录 pan.baidu.com，重载 QuickGet 扩展后再试",
        9013 => "账号受限(9013)：该账号下载功能被临时封禁，请更换账号或等待解封",
        8001 => "账号受限(8001)：同 9019，请更换账号或重新获取完整 Cookie",
        _ => return format!("百度网盘返回错误码 {errno}"),
    }
    .to_string()
}

// ---------------------------------------------------------------------------
// 登录态 / 加速档位检测。
//
// dlink 的速度档位由「解析时用的登录身份」决定：SVIP 的直链就是满速通道，
// 游客和免费账号按账号侧限额走。这里把身份显式探出来给界面展示。接口与字段
// 来自抓包：
//   POST /rest/2.0/membership/user?channel=chunlei&web=1&app_id=250528
//     → {"error_code":0,"current_product":{"cluster":"vip",
//        "detail_cluster":"svip","product_type":"vip2_1y_auto",…},…}
// ---------------------------------------------------------------------------

/// 只看 Cookie 判断是否可能已登录（不发请求）。
pub fn has_bduss(cookies: &str) -> bool {
    cookies.split(';').any(|pair| {
        let pair = pair.trim();
        match pair.split_once('=') {
            Some((name, value)) => name.trim() == "BDUSS" && value.trim().len() > 32,
            None => false,
        }
    })
}

/// 解析 membership/user 响应。error_code != 0 一律视为未登录（含 Cookie 过期）。
pub fn parse_membership(v: &Value) -> AccountInfo {
    let ok = v.get("error_code").and_then(|e| e.as_i64()) == Some(0);
    if !ok {
        return AccountInfo {
            logged_in: false,
            speed: Speed::Throttled,
            label: None,
        };
    }
    // detail_cluster: "svip" / "vip" / 空；免费账号 current_product 可能为 null。
    let cluster = v
        .pointer("/current_product/detail_cluster")
        .and_then(|c| c.as_str())
        .unwrap_or("");
    let (speed, label) = match cluster {
        "svip" | "svip10" => (Speed::Full, Some("SVIP")),
        "vip" => (Speed::Boosted, Some("会员")),
        _ => (Speed::Throttled, None),
    };
    AccountInfo {
        logged_in: true,
        speed,
        label: label.map(str::to_string),
    }
}

/// 探测当前登录身份。网络失败不致命：调用方按 Unknown 继续解析。
pub fn check_login(cookies: &str, ua: &str) -> Result<AccountInfo, String> {
    let cookies = select_cookies(cookies);
    if !has_bduss(&cookies) {
        return Ok(AccountInfo {
            logged_in: false,
            speed: Speed::Throttled,
            label: None,
        });
    }
    let client = crate::core::http::build_client(ua)?;
    let url = format!(
        "{HOST}/rest/2.0/membership/user?channel=chunlei&clienttype=0&web=1&app_id={APP_ID}"
    );
    let req = attach_cookies(
        client
            .post(url)
            .header("Referer", format!("{HOST}/"))
            .header("X-Requested-With", "XMLHttpRequest")
            .timeout(Duration::from_secs(15)),
        &cookies,
    );
    let resp = req.send().map_err(|e| format!("请求失败：{e}"))?;
    let status = resp.status();
    let text = resp.text().map_err(|e| format!("读取响应失败：{e}"))?;
    if !status.is_success() {
        return Err(format!("百度网盘返回 HTTP {status}"));
    }
    let v: Value =
        serde_json::from_str(text.trim()).map_err(|_| "百度网盘返回了非 JSON 响应".to_string())?;
    Ok(parse_membership(&v))
}

/// 完整解析：分享链接 + 浏览器 Cookie → 明文 dlink 列表。
/// 共 2~4 个小 API 调用，阻塞 IO，放到后台线程执行。
pub fn resolve_share(
    req: &NetdiskRequest,
    cookies: &str,
    ua: &str,
) -> Result<Vec<ResolvedFile>, String> {
    if !is_baidu_share_url(&req.share_url) {
        return Err(format!("不是百度网盘分享链接：{}", req.share_url));
    }
    let (tpl_surl, list_surl) = parse_surls(&req.share_url)?;
    let client = crate::core::http::build_client(ua)?;
    let cookies = select_cookies(cookies);
    let js_token = req
        .js_token
        .as_deref()
        .map(str::trim)
        .filter(|s| s.len() >= 32)
        .map(str::to_string)
        .or_else(|| fetch_html(&client, &req.share_url, &cookies).and_then(|h| extract_js_token(&h)));
    let logid = logid_from_cookies(&cookies);
    crate::core::log::write(format_args!(
        "百度解析：BDUSS={} BDCLND={} jsToken={} logid={} cookie_bytes={} ua={}",
        has_bduss(&cookies),
        bdclnd_from_cookies(&cookies).is_some(),
        js_token.is_some(),
        logid.is_some(),
        cookies.len(),
        ua
    ));

    // 1. sign + timestamp（无需登录态；带上 Cookie 也无妨）。
    let mut tpl_url = format!(
        "{HOST}/share/tplconfig?surl={tpl_surl}&fields=sign,timestamp&view_mode=1\
         &channel=chunlei&clienttype=0&web=1&app_id={APP_ID}"
    );
    let bdstoken = fetch_bdstoken(&client, &req.share_url, &cookies);
    if let Some(token) = bdstoken.as_deref() {
        tpl_url.push_str(&format!("&bdstoken={}", enc(token)));
    }
    let (sign, timestamp) = parse_tplconfig(&get_json(
        &client,
        tpl_url,
        &req.share_url,
        Some(&cookies),
    )?)
    .map_err(|e| format!("tplconfig：{e}"))?;

    // 3. 分享信息 + 文件列表。
    let mut list_url = format!(
        "{HOST}/share/list?channel=chunlei&clienttype=0&web=1&app_id={APP_ID}\
         &shorturl={list_surl}&root=1&desc=1&showempty=0&order=time&view_mode=1\
         &page=1&num=100"
    );
    if let Some(token) = bdstoken.as_deref() {
        list_url.push_str(&format!("&bdstoken={}", enc(token)));
    }
    let info = parse_share_list(&get_json(
        &client,
        list_url,
        &req.share_url,
        Some(&cookies),
    )?)
    .map_err(|e| format!("share/list：{e}"))?;

    // 选定要下的文件：显式 fids 优先，否则全部非目录条目。
    let wanted: Vec<String> = if req.fids.is_empty() {
        info.files
            .iter()
            .filter(|f| !f.is_dir)
            .map(|f| f.fs_id.clone())
            .collect()
    } else {
        req.fids.clone()
    };
    if wanted.is_empty() {
        return Err("分享里没有可下载的文件（文件夹暂不支持，请选中具体文件）".into());
    }

    // sekey：优先用调用方给的，否则从 Cookie 的 BDCLND 提取（带提取码的分享必需）。
    let sekey = req
        .sekey
        .clone()
        .or_else(|| bdclnd_from_cookies(&cookies));

    // 4. 换直链。
    let url = format!(
        "{HOST}/api/sharedownload?{}",
        request_query_full(
            &sign,
            &timestamp,
            bdstoken.as_deref(),
            logid.as_deref(),
            js_token.as_deref(),
        )
    );

    // 启用 baidu-reverse feature 时：encrypt=1 + DES 解密，直链可直接下载，无需转存。
    // 未启用时：encrypt=0 请求明文 list；若服务端仍回密文则转存回退。
    #[cfg(feature = "baidu-reverse")]
    let body = super::baidu_reverse::form_body_encrypted(
        &timestamp,
        &info.uk,
        &info.share_id,
        &wanted,
        sekey.as_deref(),
    )?;
    #[cfg(not(feature = "baidu-reverse"))]
    let body = form_body(&timestamp, &info.uk, &info.share_id, &wanted, sekey.as_deref())?;

    let (status, text) = post_form(&client, url, &req.share_url, &cookies, body)?;
    if (200..300).contains(&status) {
        match parse_response(&text) {
            Ok(files) if !files.is_empty() => {
                crate::core::log::write(format_args!(
                    "百度 sharedownload 成功，{} 个文件",
                    files.len()
                ));
                return Ok(files);
            }
            Ok(_) => {
                crate::core::log::write(format_args!(
                    "百度 sharedownload 成功但文件列表为空"
                ));
            }
            Err(e) => {
                crate::core::log::write(format_args!(
                    "百度 sharedownload 解析失败：{e}，尝试转存回退"
                ));
            }
        }
    } else {
        crate::core::log::write(format_args!(
            "百度 sharedownload HTTP {status}，尝试转存回退"
        ));
    }

    // 5. 回退：转存到网盘临时目录再走 PCS download（需要 BDUSS 登录态）。
    if has_bduss(&cookies) {
        let sizes: std::collections::HashMap<String, u64> = info
            .files
            .iter()
            .map(|f| (f.fs_id.clone(), f.size))
            .collect();
        match dlink_via_transfer(
            &client,
            &req.share_url,
            &cookies,
            bdstoken.as_deref(),
            &info.uk,
            &info.share_id,
            &wanted,
            sekey.as_deref(),
            &sizes,
        ) {
            Ok(files) if !files.is_empty() => {
                crate::core::log::write(format_args!(
                    "百度转存换链成功 {} 个文件",
                    files.len()
                ));
                return Ok(files);
            }
            Ok(_) => crate::core::log::write(format_args!(
                "百度转存换链成功但文件列表为空"
            )),
            Err(e) => crate::core::log::write(format_args!("百度转存换链失败：{e}")),
        }
    }

    Err("sharedownload 与转存换链均失败，请检查 Cookie 是否有效或稍后重试".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SHARE_URL: &str = "https://pan.baidu.com/s/1AbcDefGhiJklMnoPqrStu?pwd=xxxx";

    #[test]
    fn surl_parsing_matches_har() {
        // tplconfig 用完整 id，share/list 的 shorturl 去掉开头 '1'。
        assert_eq!(
            parse_surls(SHARE_URL).unwrap(),
            (
                "1AbcDefGhiJklMnoPqrStu".into(),
                "AbcDefGhiJklMnoPqrStu".into()
            )
        );
        assert_eq!(
            parse_surls("https://pan.baidu.com/share/init?surl=AbcDefGhiJklMnoPqrStu").unwrap(),
            (
                "AbcDefGhiJklMnoPqrStu".into(),
                "AbcDefGhiJklMnoPqrStu".into()
            )
        );
        assert!(parse_surls("https://pan.baidu.com/disk/main").is_err());
    }

    #[test]
    fn bdclnd_decode_matches_sekey_in_har() {
        // BDCLND 百分号解码后即 sekey。
        let sekey = bdclnd_from_cookies(
            "BAIDUID=x; BDCLND=ZmFrZXNla2V5MTIzNDU2Nzg5MA%3D%3D; STOKEN=y",
        )
        .unwrap();
        assert_eq!(sekey, "ZmFrZXNla2V5MTIzNDU2Nzg5MA==");
        assert_eq!(bdclnd_from_cookies("BDUSS=abc"), None);
    }

    #[test]
    fn select_cookies_keeps_login_and_share_tokens() {
        let raw = "BAIDUID=x; __bid_n=drop; BDUSS=secret; BDCLND=k%3D; Hm_lpvt_x=1; STOKEN=t";
        let kept = select_cookies(raw);
        assert!(kept.contains("BDUSS=secret"));
        assert!(kept.contains("BDCLND=k%3D"));
        assert!(kept.contains("STOKEN=t"));
        assert!(kept.contains("BAIDUID=x"));
        assert!(!kept.contains("__bid_n"));
        assert!(!kept.contains("Hm_lpvt"));
    }

    #[test]
    fn parses_tplconfig_like_har() {
        let v = json!({"errno":0,"request_id":1,"data":{"timestamp":1700000000i64,"sign":"0123456789abcdef0123456789abcdef01234567"}});
        let (sign, ts) = parse_tplconfig(&v).unwrap();
        assert_eq!(sign, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(ts, "1700000000");
        assert!(parse_tplconfig(&json!({"errno":-12})).is_err());
    }

    #[test]
    fn parses_share_list_like_har() {
        // 与抓包一致：fs_id/isdir/size 是字符串，share_id/uk 是数字。
        let v = json!({
            "errno":0,
            "title":"/DemoApp/test.zip",
            "list":[{"category":"6","fs_id":"123456789012345","isdir":"0",
                     "path":"/a/b.zip","server_filename":"test.zip","size":"147690113"},
                    {"fs_id":"999","isdir":"1","server_filename":"目录","size":"0"}],
            "share_id":10000000001i64,"uk":1000000001i64
        });
        let info = parse_share_list(&v).unwrap();
        assert_eq!(info.share_id, "10000000001");
        assert_eq!(info.uk, "1000000001");
        assert_eq!(
            info.title.as_deref(),
            Some("DemoApp/test.zip")
        );
        assert_eq!(info.files.len(), 2);
        assert!(!info.files[0].is_dir);
        assert_eq!(info.files[0].size, 147_690_113);
        assert!(info.files[1].is_dir);
    }

    #[test]
    fn body_shape_matches_working_combination() {
        let body = form_body(
            "1700000000",
            "1000000001",
            "10000000001",
            &["123456789012345".into()],
            Some("ZmFrZXNla2V5MTIzNDU2Nzg5MA=="),
        )
        .unwrap();
        // 公开版本用 encrypt=0 请求明文 list。
        // 启用 baidu-reverse feature 时由 baidu_reverse::form_body_encrypted 改用 encrypt=1。
        assert!(body.starts_with("encrypt=0&type=nolimit&product=share&timestamp=1700000000&uk=1000000001&primaryid=10000000001&fid_list=%5B123456789012345%5D"));
        assert!(
            body.contains("&extra=%7B%22sekey%22%3A%22ZmFrZXNla2V5MTIzNDU2Nzg5MA%3D%3D%22%7D"),
            "extra 编码须与抓包一致：{body}"
        );

        let open =
            form_body("1700000000", "1000000001", "10000000001", &["123".into()], None).unwrap();
        assert!(!open.contains("extra"), "免提取码分享不应带 extra：{open}");
        assert_eq!(
            form_body("1700000000", "uk", "pid", &["abc".into()], None),
            Err("无效的文件 id：abc".into())
        );
        assert_eq!(
            form_body("1700000000", "uk", "pid", &[], None),
            Err("没有可下载的文件".into())
        );
    }

    #[test]
    fn query_carries_sign_pair() {
        assert_eq!(
            request_query("0123456789abcdef0123456789abcdef01234567", "1700000000"),
            "app_id=250528&channel=chunlei&clienttype=0&web=1\
             &sign=0123456789abcdef0123456789abcdef01234567&timestamp=1700000000"
        );
        let full = request_query_full(
            "s",
            "1",
            Some("tok"),
            Some("bG9n"),
            Some("ABCDEF0123456789ABCDEF0123456789"),
        );
        assert!(full.contains("&bdstoken=tok"));
        assert!(full.contains("&logid=bG9n"));
        assert!(full.contains("&jsToken=ABCDEF0123456789ABCDEF0123456789"));
    }

    #[test]
    fn extracts_jstoken_from_share_page_html() {
        let encoded = concat!(
            "decodeURIComponent('function%20fn%28a%29%7Bwindow.jsToken%20%3D%20a%7D%3B",
            "fn%28%22ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789",
            "ABCDEF01%22')"
        );
        assert_eq!(
            extract_js_token(encoded).as_deref(),
            Some("ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF01")
        );
        assert_eq!(
            extract_js_token(r#"fn("DEADBEEFDEADBEEFDEADBEEFDEADBEEF")"#).as_deref(),
            Some("DEADBEEFDEADBEEFDEADBEEFDEADBEEF")
        );
        assert_eq!(extract_js_token("<html>no token</html>"), None);
        assert_eq!(b64_std(b"hello"), "aGVsbG8=");
        assert_eq!(
            logid_from_cookies("BAIDUID=abc:FG=1; BDUSS=x").as_deref(),
            Some(b64_std(b"abc:FG=1").as_str())
        );
    }

    #[test]
    fn parses_plaintext_dlink_list() {
        let text = concat!(
            r#"{"errno":0,"request_id":1,"server_time":1700000000,"list":["#,
            r#"{"fs_id":123456789012345,"isdir":0,"server_filename":"演示.zip","#,
            r#""size":147690113,"dlink":"https://data.d.baiduce.com/file/a?fid=x\u0026sig=y"}]}"#
        );
        let files = parse_response(text).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "演示.zip");
        assert_eq!(files[0].size, 147_690_113);
        assert_eq!(files[0].dlink, "https://data.d.baiduce.com/file/a?fid=x&sig=y");
    }

    #[test]
    fn unescapes_amp_entities_and_falls_back_to_path_name() {
        let text =
            r#"{"errno":0,"list":[{"dlink":"https://x.d.baiduce.com/a%20b.zip?b=1&amp;c=2"}]}"#;
        let files = parse_response(text).unwrap();
        assert_eq!(files[0].dlink, "https://x.d.baiduce.com/a%20b.zip?b=1&c=2");
        assert_eq!(files[0].filename, "a b.zip");
    }

    #[test]
    fn skips_dir_entries_and_missing_dlinks() {
        let text = concat!(
            r#"{"errno":0,"list":[{"isdir":1,"server_filename":"目录","#,
            r#""size":0,"dlink":""},{"server_filename":"a.zip","size":"3","dlink":"https://d/a"}]}"#
        );
        let files = parse_response(text).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename, "a.zip");
        assert_eq!(files[0].size, 3);
    }

    #[test]
    fn errno_maps_to_readable_text() {
        assert_eq!(
            errno_message(-62),
            "触发风控：请在浏览器里刷新该分享页完成验证后重试"
        );
        assert_eq!(errno_message(-12), "该分享已被取消或删除");
        assert_eq!(errno_message(999), "百度网盘返回错误码 999");
        assert!(parse_response(r#"{"errno":-33}"#)
            .unwrap_err()
            .contains("提取码"));
    }

    #[test]
    fn non_json_response_is_rejected() {
        assert!(parse_response("<html>login</html>").is_err());
        assert!(parse_response("").is_err());
        assert!(parse_response(r#"{"no_errno":1}"#).is_err());
    }

    #[test]
    fn encrypted_list_handled_gracefully() {
        // 加密 list 的处理取决于是否启用 baidu-reverse feature。
        let encrypted = r#"{"errno":0,"list":"lcLSYqsqWZI7U5ba=="}"#;
        // 无论是否启用 feature，加密 list 都不应导致 panic。
        // 未启用时返回错误提示；启用时由 baidu_reverse 模块尝试解密（此密文无效，也会报错）。
        assert!(parse_response(encrypted).is_err());

        // 空 list 字符串不应被当作密文处理
        let empty_list = r#"{"errno":0,"list":""}"#;
        assert!(parse_response(empty_list).is_err());

        assert!(response_is_encrypted(encrypted));
        assert!(!response_is_encrypted(r#"{"errno":0,"list":[{"dlink":"https://x"}]}"#));
    }

    #[test]
    fn parses_transfer_and_own_dlink_shapes() {
        let extra = json!({
            "errno":0,
            "extra":{"list":[{"from_fs_id":123456789012345i64,"to_fs_id":111,"to":"/QuickGet/a.zip"}]}
        });
        let dest = parse_transfer_dest(&extra).unwrap();
        assert_eq!(dest[0].from_fs_id, "123456789012345");
        assert_eq!(dest[0].to_fs_id.as_deref(), Some("111"));
        assert_eq!(dest[0].to_path.as_deref(), Some("/QuickGet/a.zip"));

        let info_only = json!({"errno":0,"info":[{"fsid":"111","path":"/share/a.zip"}]});
        assert!(
            parse_transfer_dest(&info_only).is_err(),
            "分享侧 info.fsid 不能当成转存目标"
        );

        let metas = json!({
            "errno":0,
            "info":[{"dlink":"https://d.pcs.baidu.com/file/a","server_filename":"a.zip","size":9}]
        });
        let files = parse_own_dlinks(&metas).unwrap();
        assert_eq!(files[0].filename, "a.zip");
        assert_eq!(files[0].size, 9);

        let dls = json!({"errno":0,"dlink":[{"dlink":"https://d/x","filename":"x.bin","size":"2"}]});
        let files = parse_own_dlinks(&dls).unwrap();
        assert_eq!(files[0].filename, "x.bin");
        assert_eq!(files[0].size, 2);
    }

    #[test]
    fn recognizes_share_urls_only() {
        assert!(is_baidu_share_url(SHARE_URL));
        assert!(is_baidu_share_url(
            "https://pan.baidu.com/share/init?surl=abcDEF"
        ));
        assert!(!is_baidu_share_url("https://pan.baidu.com/disk/main"));
        assert!(!is_baidu_share_url("https://evil.com/s/abc"));
    }

    // ---- 登录态检测（响应体取自 pan.baidu.com2.har）----

    #[test]
    fn membership_response_from_har_maps_to_svip() {
        let v = json!({
            "error_code":0,
            "current_product":{"cluster":"vip","detail_cluster":"svip",
                "product_type":"vip2_1y_auto","product_id":"9494845985462627670"},
            "level_info":{"current_level":6}
        });
        let acct = parse_membership(&v);
        assert!(acct.logged_in);
        assert_eq!(acct.speed, Speed::Full);
        assert_eq!(acct.label.as_deref(), Some("SVIP"));
    }

    #[test]
    fn membership_error_code_means_guest() {
        let v = json!({"error_code":31045,"error_msg":"user not found"});
        let acct = parse_membership(&v);
        assert!(!acct.logged_in);
        assert_eq!(acct.speed, Speed::Throttled);
        // 带了凭证但服务端不认：Cookie 失效。
        assert_eq!(acct.label, None);
    }

    #[test]
    fn free_and_vip_accounts_map_correctly() {
        let free = json!({"error_code":0,"current_product":null});
        assert_eq!(parse_membership(&free).speed, Speed::Throttled);
        let vip = json!({"error_code":0,"current_product":{"detail_cluster":"vip"}});
        let acct = parse_membership(&vip);
        assert_eq!(acct.speed, Speed::Boosted);
        assert_eq!(acct.label.as_deref(), Some("会员"));
    }

    #[test]
    fn bduss_detection_short_circuits_guest_check() {
        assert!(has_bduss(
            "BAIDUID=x; BDUSS=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; STOKEN=y"
        ));
        assert!(!has_bduss("BAIDUID=x; STOKEN=y"));
        assert!(!has_bduss("BDUSS=short"));
        assert_eq!(
            check_login("BAIDUID=x", "ua").unwrap(),
            AccountInfo {
                logged_in: false,
                speed: Speed::Throttled,
                label: None
            }
        );
    }

    #[test]
    #[ignore]
    fn live_transfer_uses_cookie_file_if_present() {
        let path = std::path::Path::new("tools/captures/.cookies");
        if !path.is_file() {
            eprintln!("skip: no cookie file");
            return;
        }
        let cookies = std::fs::read_to_string(path).unwrap();
        assert!(has_bduss(&cookies), "cookie file missing BDUSS");
        // live test 需要自行填入有效分享链接和 fs_id
        let req = NetdiskRequest {
            share_url: "https://pan.baidu.com/s/1XXXXXXXXXXXXXXXXXX?pwd=xxxx".into(),
            fids: vec!["123456789012345".into()],
            ..Default::default()
        };
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36 Edg/151.0.0.0";
        let files = Baidu.resolve(&req, &cookies, ua).expect("resolve");
        assert!(!files.is_empty(), "no files");
        assert!(
            files[0].dlink.starts_with("https://"),
            "dlink scheme"
        );
        assert!(
            files[0].dlink.contains("baidu") || files[0].dlink.contains("baidupcs"),
            "dlink host unexpected"
        );
        assert!(files[0].size > 0, "size should be known");
        eprintln!(
            "live ok n={} size={} host={}",
            files.len(),
            files[0].size,
            files[0]
                .dlink
                .split('?')
                .next()
                .unwrap_or("")
                .split('/')
                .nth(2)
                .unwrap_or("-")
        );
    }

    #[test]
    fn resolve_rejects_non_share_urls_before_network() {
        let p = Baidu {};
        let req = NetdiskRequest {
            share_url: "https://example.com/file.zip".into(),
            ..Default::default()
        };
        let err = p.resolve(&req, "", "ua").unwrap_err();
        assert!(err.contains("不是百度网盘分享链接"));
    }

    #[test]
    fn provider_trait_metadata() {
        let p = Baidu {};
        assert_eq!(p.id(), "baidu");
        assert_eq!(p.display_name(), "百度网盘");
        assert!(p.matches(SHARE_URL));
        assert!(!p.matches("https://example.com/x"));
    }
}
