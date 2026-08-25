const HOST = "com.quickget.host";
const SKIP_EXT = new Set([
  "html", "htm", "js", "css", "json", "woff", "woff2", "ttf", "svg", "ico", "map",
  "wasm", "vue", "jsx", "ts", "tsx"
]);

// Chromium 在扩展重载 / 浏览器启动时，会把下载列表里已有的任务再 fire 一遍
// onCreated。不能来一个就 cancel+投递，否则会把历史下载整锅端进 QuickGet。
const STALE_MS = 8000;
const FLUSH_MS = 450;
const REPLAY_WINDOW_MS = 2000;
const REPLAY_COOLDOWN_MS = 8000;
const DEDUPE_MS = 15000;

const workerStartedAt = Date.now();
const seenIds = new Set();
const recentUrls = new Map();
const recentCandidates = [];
let pending = [];
let flushTimer = null;
let replayUntil = 0;

async function rememberExistingDownloads() {
  try {
    const items = await chrome.downloads.search({});
    const now = Date.now();
    for (const it of items) {
      if (!it || it.id == null) continue;
      if (isRestoredDownload(it, now, workerStartedAt)) {
        seenIds.add(it.id);
      }
    }
  } catch (_) {}
}

async function onDownloadCreated(item) {
  if (!item || item.id == null) return;
  if (seenIds.has(item.id)) return;
  seenIds.add(item.id);

  const { intercept } = await chrome.storage.local.get({ intercept: false });
  if (!intercept) return;
  if (Date.now() < replayUntil) return;
  if (isRestoredDownload(item, Date.now(), workerStartedAt)) return;
  if (!shouldIntercept(item)) return;

  const url = item.finalUrl || item.url || "";
  if (wasRecentlySent(url)) return;

  noteCandidate(item);
  if (rollingLooksLikeReplay()) {
    ignoreReplayStorm(pending.length + 1);
    return;
  }

  pending.push(item);
  if (flushTimer) clearTimeout(flushTimer);
  flushTimer = setTimeout(() => {
    flushTimer = null;
    flushPending();
  }, FLUSH_MS);
}

function isRestoredDownload(item, now, workerStart) {
  if (item.state && item.state !== "in_progress") return true;
  if (item.paused) return true;
  if ((item.bytesReceived || 0) > 0) return true;
  const started = Date.parse(item.startTime || "");
  if (!started) return false;
  if (now - started > STALE_MS) return true;
  // 早于这个 service worker 启动的，是浏览器在重放旧任务。
  if (started < workerStart - 1500) return true;
  return false;
}

function noteCandidate(item) {
  const now = Date.now();
  recentCandidates.push({
    t: now,
    host: hostOf(item.finalUrl || item.url),
    ref: hostOf(item.referrer)
  });
  while (recentCandidates.length && now - recentCandidates[0].t > REPLAY_WINDOW_MS) {
    recentCandidates.shift();
  }
}

function rollingLooksLikeReplay() {
  return looksLikeHistoryReplay(recentCandidates);
}

function looksLikeHistoryReplay(entries) {
  if (entries.length < 6) return false;
  const hosts = unique(entries.map((e) => e.host));
  const refs = unique(entries.map((e) => e.ref));
  if (hosts.size >= 4) return true;
  if (refs.size >= 3 && entries.length >= 8) return true;
  if (entries.length >= 20 && hosts.size >= 2) return true;
  return false;
}

function ignoreReplayStorm(n) {
  pending = [];
  if (flushTimer) {
    clearTimeout(flushTimer);
    flushTimer = null;
  }
  replayUntil = Date.now() + REPLAY_COOLDOWN_MS;
  reportReplay(n);
}

async function flushPending() {
  const items = pending.splice(0);
  if (!items.length) return;
  if (Date.now() < replayUntil || looksLikeHistoryReplay(recentCandidates)) {
    reportReplay(items.length);
    return;
  }
  const names = [];
  let lastError = null;
  for (const item of items) {
    const result = await stealAndSend(item);
    if (result && result.ok && result.filename) {
      names.push(result.filename);
    } else if (result && result.ok === false) {
      lastError = result;
    }
  }
  if (names.length === 1) {
    reportCapture({ ok: true }, names[0]);
  } else if (names.length > 1) {
    reportCapture({ ok: true }, `${names.length} 个文件`);
  } else if (lastError) {
    reportCapture(lastError);
  }
}

async function stealAndSend(item) {
  try {
    const found = await chrome.downloads.search({ id: item.id });
    const fresh = found && found[0];
    if (fresh && fresh.state === "complete") return { skipped: true };
  } catch (_) {
    return { skipped: true };
  }
  const url = item.finalUrl || item.url;
  const filename = item.filename ? basename(item.filename) : filenameFromUrl(url);
  const result = await sendJob({
    url,
    referer: item.referrer || "",
    filename
  }, null, url, false);
  if (!result || !result.ok) {
    return { ok: false, error: (result && result.error) || "QuickGet 未运行，请先打开应用" };
  }
  try {
    await chrome.downloads.cancel(item.id);
    await chrome.downloads.erase({ id: item.id });
  } catch (_) {}
  markSent(url);
  return { ok: true, filename };
}

async function sendJob(partial, tab, cookieUrl, notify) {  const url = partial.url;
  if (!url || url.startsWith("blob:") || url.startsWith("data:") || url.startsWith("chrome")) {
    return { ok: false, error: "unsupported url" };
  }
  const cookies = await cookieHeader(cookieUrl || url);
  const ua = navigator.userAgent;
  const payload = {
    url,
    cookies: cookies || "",
    referer: partial.referer || (tab && tab.url) || "",
    ua,
    filename: partial.filename || ""
  };
  const result = await nativeSend(payload);
  if (notify) {
    reportCapture(result, payload.filename || filenameFromUrl(url));
  }
  return result;
}

// 网盘分享页入口：只传分享链接，直链解析由 QuickGet 主程序的 providers 插件完成。
// Cookie 在这里附上（登录态、提取码凭证），但只用于网盘域内的解析 API。
// 新增支持一家网盘：往 NETDISK_HOSTS 加主机名 + manifest 的 content_scripts。
const NETDISK_HOSTS = ["pan.baidu.com"];

function isNetdiskShareUrl(url) {
  try {
    const u = new URL(url);
    return NETDISK_HOSTS.includes(u.hostname);
  } catch (_) {
    return false;
  }
}

async function sendNetdiskShare(shareUrl, jsToken) {
  if (!isNetdiskShareUrl(shareUrl)) {
    return { ok: false, error: "不是支持的网盘分享链接" };
  }
  const cookies = await cookieHeader("https://" + new URL(shareUrl).hostname + "/");
  const netdisk = { share_url: shareUrl };
  if (jsToken) netdisk.js_token = jsToken;
  const payload = {
    url: shareUrl,
    referer: shareUrl,
    ua: navigator.userAgent,
    filename: "",
    cookies,
    netdisk
  };
  const result = await nativeSend(payload);
  if (result && result.ok) {
    flashBadge(true);
  }
  return result || { ok: false, error: "QuickGet 未运行，请先打开应用" };
}

function reportCapture(result, filename) {
  const ok = !!(result && result.ok);
  const name = filename || "download";
  const message = ok
    ? `已交给 QuickGet：${name}`
    : (result && result.error) || "QuickGet 未运行，请先打开应用";
  flashBadge(ok);
  notify(message);
}

function reportReplay(n) {
  flashBadge(false);
  notify(`浏览器在恢复历史下载，已跳过 ${n} 项，没有交给 QuickGet`);
}

function notify(message) {
  try {
    chrome.notifications.create({
      type: "basic",
      iconUrl: "icons/128.png",
      title: "QuickGet",
      message
    });
  } catch (_) {}
}

function flashBadge(ok) {
  try {
    chrome.action.setBadgeBackgroundColor({ color: ok ? "#0B6E4F" : "#BA1A1A" });
    chrome.action.setBadgeText({ text: ok ? "OK" : "!" });
    setTimeout(() => {
      chrome.action.setBadgeText({ text: "" });
    }, 4000);
  } catch (_) {}
}

// 本地 HTTP 通道：QuickGet GUI 运行时监听。优先走这里——
// 部分机器上 Chrome/Edge 的原生消息通道会被安全策略卡住（表现为永远超时），
// HTTP 完全绕开那套注册表 + 宿主进程机制。
const HTTP_ENDPOINT = "http://127.0.0.1:18666";

function nativeSend(payload) {
  return httpSend(payload).then((r) => r || nativeSendViaHost(payload));
}

async function httpSend(payload) {
  try {
    const resp = await fetch(HTTP_ENDPOINT + "/job", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload)
    });
    if (!resp.ok) return null;
    return await resp.json();
  } catch (_) {
    // GUI 没在跑：静默回退到 Native Messaging（它能顺便把 GUI 拉起来）。
    return null;
  }
}

function nativeSendViaHost(payload) {
  return new Promise((resolve) => {
    chrome.runtime.sendNativeMessage(HOST, payload, (resp) => {
      if (chrome.runtime.lastError) {
        resolve({ ok: false, error: chrome.runtime.lastError.message });
        return;
      }
      resolve(resp || { ok: true });
    });
  });
}

async function cookieHeader(url) {
  try {
    const list = await chrome.cookies.getAll({ url });
    if (!list || !list.length) return "";
    return list.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch (_) {
    return "";
  }
}

function shouldIntercept(item) {
  const url = item.finalUrl || item.url || "";
  if (!url.startsWith("http://") && !url.startsWith("https://")) return false;
  const name = (item.filename || filenameFromUrl(url)).toLowerCase();
  const ext = name.split(".").pop();
  if (SKIP_EXT.has(ext)) return false;
  if (item.mime && (item.mime.startsWith("text/html") || item.mime === "application/wasm")) {
    return false;
  }
  return true;
}

function wasRecentlySent(url) {
  pruneRecentUrls();
  const key = identityKey(url);
  return recentUrls.has(key);
}

function markSent(url) {
  pruneRecentUrls();
  recentUrls.set(identityKey(url), Date.now());
}

function pruneRecentUrls() {
  const now = Date.now();
  for (const [key, ts] of recentUrls) {
    if (now - ts > DEDUPE_MS) recentUrls.delete(key);
  }
}

function identityKey(url) {
  try {
    const u = new URL(url);
    u.hash = "";
    u.search = "";
    return u.toString();
  } catch (_) {
    return String(url || "");
  }
}

function hostOf(url) {
  try {
    return new URL(url || "").host.toLowerCase();
  } catch (_) {
    return "";
  }
}

function unique(values) {
  return new Set((values || []).filter(Boolean));
}

function filenameFromUrl(url) {
  try {
    const u = new URL(url);
    const last = u.pathname.split("/").filter(Boolean).pop() || "download.bin";
    return decodeURIComponent(last);
  } catch (_) {
    return "download.bin";
  }
}

function basename(p) {
  return String(p).split(/[\\/]/).pop();
}

function bindBrowser() {
  rememberExistingDownloads();

  chrome.runtime.onInstalled.addListener(() => {
    chrome.contextMenus.removeAll(() => {
      chrome.contextMenus.create({
        id: "qg-link",
        title: "用 QuickGet 下载链接",
        contexts: ["link"]
      });
      chrome.contextMenus.create({
        id: "qg-page",
        title: "用 QuickGet 下载本页",
        contexts: ["page"]
      });
      chrome.contextMenus.create({
        id: "qg-media",
        title: "用 QuickGet 下载",
        contexts: ["image", "video", "audio"]
      });
    });
  });

  chrome.contextMenus.onClicked.addListener(async (info, tab) => {
    const url = info.linkUrl || info.srcUrl || info.pageUrl || (tab && tab.url);
    if (!url) return;
    await sendJob({
      url,
      referer: (tab && tab.url) || info.pageUrl || "",
      filename: filenameFromUrl(url)
    }, tab, null, true);
  });

  chrome.downloads.onCreated.addListener((item) => {
    onDownloadCreated(item).catch(() => {});
  });

  chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
    if (msg && msg.type === "download-tab") {
      chrome.tabs.query({ active: true, currentWindow: true }, async (tabs) => {
        const tab = tabs[0];
        if (!tab || !tab.url) {
          sendResponse({ ok: false, error: "no tab" });
          return;
        }
        const result = await sendJob({ url: tab.url, referer: tab.url }, tab);
        sendResponse(result);
      });
      return true;
    }
    // v0.1.2 用的是 quickget-baidu，保留兼容一个版本周期。
    if (msg && (msg.type === "quickget-netdisk" || msg.type === "quickget-baidu")) {
      sendNetdiskShare(String(msg.shareUrl || ""), String(msg.jsToken || "")).then(sendResponse);
      return true;
    }
    if (msg && msg.type === "open-app") {
      nativeSend({ ping: true }).then((resp) => {
        if (chrome.runtime.lastError) {
          sendResponse({ ok: false, error: chrome.runtime.lastError.message });
          return;
        }
        sendResponse(resp || { ok: true });
      });
      return true;
    }
    return false;
  });
}

if (typeof chrome !== "undefined" && chrome.runtime) {
  bindBrowser();
}

if (typeof module !== "undefined") {
  module.exports = {
    isRestoredDownload,
    looksLikeHistoryReplay,
    shouldIntercept
  };
}
