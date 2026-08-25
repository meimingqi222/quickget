//! 网盘分享解析的插件式抽象层。
//!
//! 各家网盘（百度、夸克、阿里…）的分享直链协议差异很大，但对外能力是同构的：
//!   1. 认领链接：这个 URL 归不归我管；
//!   2. 探测登录档位：决定直链的服务器限速水平，给用户可见的加速反馈；
//!   3. 解析：分享链接 + 浏览器 Cookie → 一批带签名的明文直链。
//!
//! 上层（收件箱、GUI、HTTP 引擎）只依赖这里的 trait 与结构体。新增一家网盘：
//! 在本目录新建模块实现 `ShareProvider`，往 `PROVIDERS` 表里加一行即可，
//! 扩展侧再往 manifest 的 content_scripts 与 background 的域名白名单加个主机名。

pub mod baidu;
#[cfg(feature = "baidu-reverse")]
pub mod baidu_reverse;

use serde::{Deserialize, Serialize};

/// 收件箱里的网盘任务请求。`provider` 留空时按 share_url 自动认领；
/// 兼容旧版 `baidu` 字段（serde alias）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetdiskRequest {
    /// 提供商 id（如 "baidu"）。None = 自动识别。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// 分享页 URL，也是解析请求的 Referer。
    pub share_url: String,
    /// 只下载这些文件 id。空 = 全部普通文件。id 的形态由各家自定义。
    #[serde(default)]
    pub fids: Vec<String>,
    /// 提取码/访问凭证覆盖；一般留空，由各家从 Cookie 自行提取。
    #[serde(default)]
    pub sekey: Option<String>,
    /// 分享页 window.jsToken。百度 sharedownload 不带它会回加密 list。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub js_token: Option<String>,
}

/// 解析产物：一个可直接下载的文件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedFile {
    pub dlink: String,
    pub filename: String,
    pub size: u64,
    /// 转存到网盘的路径（如有）。下载完成后用于清理网盘临时文件。
    /// None = 非转存方式获取的直链（如 sharedownload），无需清理。
    pub transfer_path: Option<String>,
}

/// 该身份下直链的服务端限速水平。这是账号权益决定的，
/// 客户端无法绕过——这里把它显式化是为了给用户可见的预期。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Speed {
    /// 探测失败 / 未探测。
    #[default]
    Unknown,
    /// 游客或免费档：严格限速。
    Throttled,
    /// 付费会员：有限加速。
    Boosted,
    /// 顶级会员（SVIP 等）：基本满速。
    Full,
}

/// 登录探测结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountInfo {
    pub logged_in: bool,
    pub speed: Speed,
    /// 展示名（"SVIP"/"会员"/"88VIP"…）。未登录或未知为 None。
    pub label: Option<String>,
}

/// 网盘提供商插件接口。全部方法阻塞网络 IO，调用方需放到后台线程；
/// 除 `id/display_name/matches` 外都可能失败，错误文案应面向用户。
pub trait ShareProvider: Sync {
    /// 稳定标识，收件箱消息与日志用它路由。
    fn id(&self) -> &'static str;
    /// 界面展示名。
    fn display_name(&self) -> &'static str;
    /// 这个链接是否归本提供商处理（仅凭 URL 判断，不发请求）。
    fn matches(&self, url: &str) -> bool;
    /// 登录档位探测。失败不致命：调用方按 Unknown 继续解析。
    fn check_login(&self, cookies: &str, ua: &str) -> Result<AccountInfo, String>;
    /// 核心：分享链接 → 明文直链列表。
    fn resolve(
        &self,
        req: &NetdiskRequest,
        cookies: &str,
        ua: &str,
    ) -> Result<Vec<ResolvedFile>, String>;
    /// 真正拉 dlink 时用的 UA。None = 沿用解析时的浏览器 UA。
    /// 百度 >20MB 的直链要求非浏览器 UA，所以这里单独覆盖。
    fn download_ua(&self) -> Option<&'static str> {
        None
    }
    /// 单个 Range 请求的最大字节数。None = 不限制（用引擎默认分片）。
    /// 百度 PCS 直链对单次 Range > 4MB 回 31326 hitcode:104 风控，必须限制。
    fn max_part_size(&self) -> Option<u64> {
        None
    }
}

/// 插件注册表。新增网盘在这里加一行。
const PROVIDERS: &[&dyn ShareProvider] = &[&baidu::Baidu];

/// 按 id 或 URL 找提供商。显式 id 优先；找不到返回 None。
pub fn find(provider: Option<&str>, url: &str) -> Option<&'static dyn ShareProvider> {
    if let Some(id) = provider {
        return PROVIDERS.iter().copied().find(|p| p.id() == id);
    }
    PROVIDERS.iter().copied().find(|p| p.matches(url))
}

/// 是否有任何已注册提供商认领该链接。
pub fn is_supported_url(url: &str) -> bool {
    PROVIDERS.iter().any(|p| p.matches(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_routes_by_url_and_id() {
        let p = find(None, "https://pan.baidu.com/s/1AbcDefGhiJklMnoPqrStu").unwrap();
        assert_eq!(p.id(), "baidu");
        assert_eq!(p.display_name(), "百度网盘");
        assert_eq!(
            p.download_ua(),
            Some("netdisk;P2SP;3.0.0.8;netdisk;11.12.3;ANG-AN00;android-android;10.0;JSbridge4.4.0;jointBridge;1.1.0;")
        );

        assert!(find(Some("baidu"), "https://whatever.example/x").is_some());
        assert!(find(None, "https://example.com/file.zip").is_none());
        assert!(find(Some("quark"), "https://pan.baidu.com/s/abc").is_none());
    }

    #[test]
    fn request_deserializes_legacy_baidu_field() {
        // 旧版本收件箱里可能残留 "baidu" 键的任务，不能丢。
        let legacy = serde_json::json!({
            "baidu": {"share_url": "https://pan.baidu.com/s/abc", "fids": ["1"]}
        });
        let v: serde_json::Value = legacy;
        let job: NetdiskRequest =
            serde_json::from_value(v.get("baidu").unwrap().clone()).unwrap();
        assert_eq!(job.share_url, "https://pan.baidu.com/s/abc");
        assert_eq!(job.fids, vec!["1"]);
        assert_eq!(job.provider, None);
    }

    #[test]
    fn request_round_trips_provider_field() {
        let req = NetdiskRequest {
            provider: Some("baidu".into()),
            share_url: "https://pan.baidu.com/s/abc".into(),
            fids: vec![],
            sekey: None,
            js_token: None,
        };
        let text = serde_json::to_string(&req).unwrap();
        assert!(!text.contains("baidu\":"), "provider=None 时不应序列化: {text}");
        let back: NetdiskRequest = serde_json::from_str(&text).unwrap();
        assert_eq!(back, req);
    }
}
