// process 插件模板（协议 2），Node.js 18 以上，不需要安装依赖。
//
// 软件运行 `node main.js <方法>`，把请求 JSON 写进标准输入：
//   {"protocol": 2, "method": "...", "params": {...}, "settings": {...}, "context": {...}}
// 插件往标准输出一行写一个事件（log / progress / result / error），最后一行是 result 或 error。
//
// 这个模板不联网，search 返回假数据。改成真的时用 fetch() 请求 permissions.hosts 里的网站，
// resolve 给出的下载地址也必须在 hosts 里。
"use strict";

const BASE = "https://example.org";

function emit(event, data) {
  process.stdout.write(JSON.stringify({ event, ...data }) + "\n");
}
const log = (msg, level = "info") => emit("log", { level, msg });

class PluginError extends Error {
  constructor(code, message) {
    super(message);
    this.code = code;
  }
}

const methods = {
  info() {
    return { ok: true, message: "模板插件可以运行" };
  },

  async search({ query, page, media_kind }, settings, context) {
    query = (query || "").trim();
    if (!query || (page || 1) > 1) return [];
    // 真实插件：
    // const r = await fetch(`${BASE}/search?q=${encodeURIComponent(query)}`);
    // if (r.status === 401) throw new PluginError("login_required", "需要先在“网站登录”里登录");
    const items = [1, 2, 3].map((i) => ({
      id: `demo-${i}`,
      title: `${query} 示例 ${i}`,
      artist: "示例作者",
      media_kind: media_kind || "video",
      qualities: [
        { key: "1080", label: "1080P", format: "mp4" },
        { key: "720", label: "720P", format: "mp4" },
      ],
      extra: { page: `${BASE}/watch/demo-${i}` },
    }));
    log(`找到 ${items.length} 条`);
    return items;
  },

  resolve({ item, quality }, settings) {
    const page = item && item.extra && item.extra.page;
    if (!page) throw new PluginError("not_found", "这条结果没有地址");
    const h = quality || settings.quality || "1080";
    // 交给软件自带的 yt-dlp 下载：只能给页面地址和格式选择（-f），不能加别的参数
    return { target: { type: "ytdlp", url: page, format: `bv*[height<=${h}]+ba/b[height<=${h}]` } };
  },
};

async function main() {
  const method = process.argv[2] || "";
  let raw = "";
  for await (const chunk of process.stdin) raw += chunk;
  const req = raw.trim() ? JSON.parse(raw) : {};
  if (req.protocol !== 2) {
    process.stderr.write("这个插件需要协议 2（0.5.0 以上版本的软件）\n");
    process.exit(2);
  }
  const fn = methods[method];
  if (!fn) return emit("error", { code: "unsupported", message: `不支持的方法：${method}` });
  try {
    const data = await fn(req.params || {}, req.settings || {}, req.context || {});
    emit("result", { data: data === undefined ? null : data });
  } catch (e) {
    emit("error", { code: e.code || "internal", message: String(e.message || e) });
  }
}

main();
