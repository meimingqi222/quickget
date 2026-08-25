// 网盘分享页（manifest 里声明的各网盘域名）注入的入口按钮。
//
// 职责刻意保持极小且厂商无关：只把「当前页面链接」交给 background，
// 由 QuickGet 主程序里的 providers 插件完成直链解析。不读取页面内部
// 变量、不解析 DOM 列表——那些都会随各家改版失效。
// 新增支持一家网盘：manifest 的 content_scripts.matches 加域名即可，
// 本文件无需改动。
(() => {
  "use strict";
  if (window.__quickgetNetdiskLoaded) return;
  window.__quickgetNetdiskLoaded = true;

  const BTN_ID = "qg-netdisk-share-btn";

  function toast(message, ok) {
    const old = document.getElementById("qg-netdisk-toast");
    if (old) old.remove();
    const el = document.createElement("div");
    el.id = "qg-netdisk-toast";
    el.textContent = message;
    Object.assign(el.style, {
      position: "fixed",
      left: "50%",
      bottom: "96px",
      transform: "translateX(-50%)",
      zIndex: "2147483647",
      maxWidth: "70vw",
      padding: "10px 16px",
      borderRadius: "8px",
      background: ok ? "#0B6E4F" : "#BA1A1A",
      color: "#fff",
      fontSize: "13px",
      lineHeight: "1.5",
      boxShadow: "0 4px 16px rgba(0,0,0,.25)",
      fontFamily: "system-ui, sans-serif"
    });
    document.documentElement.appendChild(el);
    setTimeout(() => el.remove(), ok ? 3500 : 6000);
  }

  function ensureButton() {
    if (document.getElementById(BTN_ID)) return;
    const btn = document.createElement("div");
    btn.id = BTN_ID;
    btn.textContent = "QuickGet 下载";
    btn.title = "把这个分享交给 QuickGet 下载";
    Object.assign(btn.style, {
      position: "fixed",
      right: "18px",
      bottom: "84px",
      zIndex: "2147483646",
      padding: "9px 18px",
      borderRadius: "22px",
      background: "#06A7FF",
      color: "#fff",
      fontSize: "13px",
      fontWeight: "600",
      cursor: "pointer",
      userSelect: "none",
      boxShadow: "0 2px 12px rgba(6,167,255,.45)",
      fontFamily: "system-ui, sans-serif"
    });
    btn.addEventListener("mouseenter", () => (btn.style.filter = "brightness(1.08)"));
    btn.addEventListener("mouseleave", () => (btn.style.filter = ""));
    function readJsToken() {
      const html = document.documentElement.innerHTML || "";
      const m =
        html.match(/fn\("([A-Fa-f0-9]{32,})"\)/) ||
        html.match(/fn%28%22([A-Fa-f0-9]{32,})%22/);
      return m ? m[1] : "";
    }

    btn.addEventListener("click", () => {
      const shareUrl = location.origin + location.pathname + location.search;
      btn.textContent = "发送中…";
      btn.style.pointerEvents = "none";
      let settled = false;
      try {
        chrome.runtime.sendMessage(
          { type: "quickget-netdisk", shareUrl, jsToken: readJsToken() },
          (resp) => {
            settled = true;
            const ok = !!(resp && resp.ok);
            restore(ok ? "已交给 QuickGet" : "QuickGet 下载");
            if (!ok) {
              toast((resp && resp.error) || "QuickGet 未运行，请先打开应用", false);
            }
          }
        );
      } catch (_) {
        restore("QuickGet 下载");
        toast("无法连接 QuickGet 扩展", false);
        return;
      }
      // sendMessage 没回调（宿主不在）时也要恢复按钮。
      setTimeout(() => {
        if (!settled) {
          restore("QuickGet 下载");
          toast("QuickGet 未响应，请先打开应用", false);
        }
      }, 4000);
    });

    function restore(label) {
      btn.textContent = label;
      btn.style.pointerEvents = "";
    }

    document.documentElement.appendChild(btn);
  }

  ensureButton();
})();
