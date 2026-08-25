const {
  isRestoredDownload,
  looksLikeHistoryReplay,
  shouldIntercept
} = require("./background.js");

function assert(cond, msg) {
  if (!cond) {
    throw new Error(msg);
  }
}

const now = Date.parse("2026-08-25T17:29:42Z");
const workerStart = now;

assert(
  isRestoredDownload(
    { state: "in_progress", startTime: new Date(now - 60_000).toISOString(), bytesReceived: 0 },
    now,
    workerStart
  ),
  "minute-old startTime is restored"
);

assert(
  isRestoredDownload(
    { state: "in_progress", startTime: new Date(now).toISOString(), bytesReceived: 12_000 },
    now,
    workerStart
  ),
  "already received bytes is restored"
);

assert(
  !isRestoredDownload(
    { state: "in_progress", startTime: new Date(now).toISOString(), bytesReceived: 0 },
    now,
    workerStart
  ),
  "fresh click is not restored"
);

assert(
  looksLikeHistoryReplay([
    { host: "github.com", ref: "github.com" },
    { host: "dl.google.com", ref: "google.com" },
    { host: "pan.baidu.com", ref: "pan.baidu.com" },
    { host: "jetbrains.com", ref: "jetbrains.com" },
    { host: "takeout.google.com", ref: "" },
    { host: "quark.cn", ref: "quark.cn" }
  ]),
  "mixed hosts in 2s is history replay"
);

assert(
  !looksLikeHistoryReplay(
    Array.from({ length: 8 }, () => ({ host: "takeout.google.com", ref: "takeout.google.com" }))
  ),
  "same-site takeout burst is not replay"
);

assert(!shouldIntercept({ url: "https://a.com/app.wasm" }), "wasm skipped");
assert(!shouldIntercept({ url: "https://a.com/index.vue" }), "vue skipped");
assert(shouldIntercept({ url: "https://a.com/app.dmg" }), "dmg kept");

console.log("ok");
