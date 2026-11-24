// js 沙箱插件模板（协议 2）。整个插件就是这一个 ES 模块文件：
// 沙箱里没有 import / require、文件、定时器（setTimeout）、fetch，只有软件给的 `mp`。
// 用到第三方库时先用 esbuild / rollup 打成一个文件。
//
// 每个方法是一个导出函数，参数是 (params, mp)，返回值就是结果；可以是 async。
// 出错时 throw new PluginError("错误码", "给用户看的话")。

const BASE = "https://example.org";

// 搜索：返回 Item 数组（作品；有 episodes 能力时软件会把前几部展开成各话）
export function search({ query, page }, mp) {
  const r = mp.http.get(BASE + "/search?" + mp.url.query({ q: query, page, lang: mp.settings.lang }));
  const rows = mp.html.select(r.text, ".book-item");
  mp.log("找到", rows.length, "条");
  return rows.map((row) => {
    const a = mp.html.selectOne(row.html, "a");
    const img = mp.html.selectOne(row.html, "img");
    const author = mp.html.selectOne(row.html, ".author");
    const href = mp.url.resolve(r.url, a ? a.attrs.href : "");
    return {
      id: href,
      title: a ? a.text : "",
      series: a ? a.text : "",
      artist: author ? author.text : "",
      cover: img ? mp.url.resolve(r.url, img.attrs.src || "") : "",
      extra: { url: href },
    };
  });
}

// 一部作品 → 各话
export function episodes({ item }, mp) {
  const r = mp.http.get(item.extra.url);
  return mp.html.select(r.text, "#chapters li a").map((a) => {
    const m = /第\s*(\d+(?:\.\d+)?)\s*话/.exec(a.text);
    return {
      id: mp.url.resolve(r.url, a.attrs.href),
      title: a.text,
      episode: m ? Number(m[1]) : undefined,
      extra: { url: mp.url.resolve(r.url, a.attrs.href) },
    };
  });
}

// 一话 → 全部图片地址（软件按顺序下载后打包成 CBZ）
export function resolve({ item }, mp) {
  const r = mp.http.get(item.extra.url, { headers: { Referer: BASE + "/" } });
  const urls = mp.html
    .select(r.text, ".reader img")
    .map((img) => mp.url.resolve(r.url, img.attrs["data-src"] || img.attrs.src || ""))
    .filter((u) => u);
  if (!urls.length) throw new PluginError("not_found", "这一话没有图片");
  return { target: { type: "pages", urls, headers: { Referer: BASE + "/" } } };
}
