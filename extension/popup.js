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
