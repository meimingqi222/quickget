const statusEl = document.getElementById("status");
const interceptEl = document.getElementById("intercept");

chrome.storage.local.get({ intercept: false }, (v) => {
  interceptEl.checked = !!v.intercept;
});

interceptEl.addEventListener("change", () => {
  chrome.storage.local.set({ intercept: interceptEl.checked });
});

document.getElementById("send").addEventListener("click", () => {
  statusEl.className = "";
  statusEl.textContent = "发送中…";
  chrome.runtime.sendMessage({ type: "download-tab" }, (resp) => {
    if (chrome.runtime.lastError) {
      statusEl.className = "err";
      statusEl.textContent = chrome.runtime.lastError.message;
      return;
    }
    if (resp && resp.ok) {
      statusEl.textContent = "已交给 QuickGet";
    } else {
      statusEl.className = "err";
      statusEl.textContent = (resp && resp.error) || "QuickGet 未运行，请先打开应用";
    }
  });
});

document.getElementById("open").addEventListener("click", () => {
  statusEl.className = "";
  statusEl.textContent = "正在打开…";
  chrome.runtime.sendMessage({ type: "open-app" }, (resp) => {
    if (chrome.runtime.lastError) {
      statusEl.className = "err";
      statusEl.textContent = chrome.runtime.lastError.message;
      return;
    }
    if (resp && resp.ok) {
      statusEl.textContent = "已请求打开 QuickGet";
    } else {
      statusEl.className = "err";
      statusEl.textContent = (resp && resp.error) || "QuickGet 未运行，请先打开应用";
    }
  });
});

// 复制百度网盘 Cookie 到剪贴板，供 baidu-share admin 页面粘贴。
// 只保留 quickgetd 解析实际需要的 3 个核心字段（经实测验证）：
//   BDUSS  - 登录态核心凭证，转存/PCS download 必需
//   BAIDUID - logid 来源（分享页 boot 脚本用它生成 logid 参数）
//   STOKEN  - 安全令牌，tplconfig 需要它验证会话（缺了会被重定向到登录页）
// BDCLND 不需要预先提供：带提取码的分享 verify 时会自动 Set-Cookie 生成。
// 其余 20+ 个字段（Hm_lvt_*、H_PS_PSSID、ab_sr、PANPSC 等）是统计/追踪 cookie，与下载无关。
// 需要 manifest 里的 cookies 权限 + host_permissions。
const BAIDU_COOKIE_KEEP = new Set(["BDUSS", "BAIDUID", "STOKEN"]);

document.getElementById("copy-baidu-cookie").addEventListener("click", async () => {
  statusEl.className = "";
  statusEl.textContent = "正在读取 Cookie…";
  try {
    const list = await chrome.cookies.getAll({ url: "https://pan.baidu.com/" });
    if (!list || !list.length) {
      statusEl.className = "err";
      statusEl.textContent = "未找到百度网盘 Cookie，请先在浏览器登录 pan.baidu.com";
      return;
    }
    const hasBDUSS = list.some((c) => c.name === "BDUSS" && c.value);
    if (!hasBDUSS) {
      statusEl.className = "err";
      statusEl.textContent = "Cookie 里没有 BDUSS，请先在浏览器登录百度网盘";
      return;
    }
    const kept = list.filter((c) => BAIDU_COOKIE_KEEP.has(c.name) && c.value);
    const header = kept.map((c) => `${c.name}=${c.value}`).join("; ");
    await navigator.clipboard.writeText(header);
    statusEl.textContent = `已复制 ${kept.length} 个关键字段到剪贴板，粘贴到 admin 页面即可`;
  } catch (e) {
    statusEl.className = "err";
    statusEl.textContent = String(e && e.message || e || "复制失败");
  }
});
