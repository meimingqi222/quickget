const HOST = "com.quickget.host";
const SKIP_EXT = new Set([
  "html", "htm", "js", "css", "json", "woff", "woff2", "ttf", "svg", "ico", "map"
]);

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
  }, tab);
});

chrome.downloads.onCreated.addListener(async (item) => {
  const { intercept } = await chrome.storage.local.get({ intercept: false });
  if (!intercept) return;
  if (!shouldIntercept(item)) return;
  try {
    await chrome.downloads.cancel(item.id);
    await chrome.downloads.erase({ id: item.id });
  } catch (_) {
    return;
  }
  const tab = item.finalUrl || item.url;
  await sendJob({
    url: item.finalUrl || item.url,
    referer: item.referrer || "",
    filename: item.filename ? basename(item.filename) : filenameFromUrl(item.finalUrl || item.url)
  }, null, tab);
});

async function sendJob(partial, tab, cookieUrl) {
  const url = partial.url;
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
  return nativeSend(payload);
}

function nativeSend(payload) {
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
  if (item.mime && item.mime.startsWith("text/html")) return false;
  return true;
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
});
