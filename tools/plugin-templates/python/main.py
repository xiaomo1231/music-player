"""process 插件模板（协议 2），只用 Python 标准库。

软件运行 `python main.py <方法>`，把请求 JSON 写进标准输入：
    {"protocol": 2, "method": "...", "params": {...}, "settings": {...}, "context": {...}}
插件往标准输出一行写一个事件（log / progress / result / error），最后一行是 result 或 error。

这个模板不联网，search 返回假数据，方便先跑通。改成真的时：
- 用 http_get() 请求 permissions.hosts 里声明的网站；
- resolve 给出的下载地址也必须在 hosts 里，否则软件拒绝下载。
"""
import json
import sys
import urllib.parse
import urllib.request

BASE = "https://example.org"


def emit(event, **kw):
    kw["event"] = event
    sys.stdout.buffer.write((json.dumps(kw, ensure_ascii=False) + "\n").encode("utf-8"))
    sys.stdout.flush()


def log(msg, level="info"):
    emit("log", level=level, msg=msg)


def fail(code, message):
    """code 用约定的错误码：login_required / captcha / forbidden / not_found /
    rate_limited / network / bad_config / unsupported / internal"""
    emit("error", code=code, message=message)
    sys.exit(0)


def http_get(url, context, headers=None):
    """请求网页。登录过的网站，软件会把用户保存的 Cookie 放在 context.cookies[域名] 里
    （只有用户同意交出登录时才有），插件不要自己去登录。"""
    host = urllib.parse.urlsplit(url).hostname or ""
    h = {"User-Agent": "MediaPlayerPlugin/0.1", **(headers or {})}
    cookie = (context.get("cookies") or {}).get(host)
    if cookie:
        h["Cookie"] = cookie
    req = urllib.request.Request(url, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=30) as r:
            return r.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        if e.code in (401,):
            fail("login_required", f"需要先在“网站登录”里登录 {host}")
        if e.code == 403:
            fail("forbidden", f"网站拒绝访问（HTTP 403）")
        if e.code == 404:
            fail("not_found", "网站上没有这个内容")
        if e.code == 429:
            fail("rate_limited", "请求太频繁，过一会儿再试")
        fail("network", f"HTTP {e.code}")
    except OSError as e:
        fail("network", f"连接失败：{e}")


def info(params, settings, context):
    return {"ok": True, "message": "模板插件可以运行"}


def search(params, settings, context):
    query = (params.get("query") or "").strip()
    if not query or int(params.get("page") or 1) > 1:
        return []
    limit = int(settings.get("max_results") or 20)
    # 真实插件：html = http_get(f"{BASE}/search?q={urllib.parse.quote(query)}", context)，再解析
    items = []
    for i in range(1, min(limit, 3) + 1):
        items.append({
            "id": f"demo-{i}",
            "title": f"{query} 示例 {i}",
            "artist": "示例艺人",
            "album": "示例专辑",
            "media_kind": params.get("media_kind") or "music",
            "qualities": [
                {"key": "flac", "label": "无损", "format": "flac"},
                {"key": "mp3", "label": "320K", "format": "mp3", "bitrate": 320},
            ],
            "extra": {"path": f"/files/demo-{i}"},
        })
    log(f"找到 {len(items)} 条")
    return items


def resolve(params, settings, context):
    item = params.get("item") or {}
    quality = params.get("quality") or "flac"
    ext = "mp3" if quality == "mp3" else "flac"
    path = (item.get("extra") or {}).get("path")
    if not path:
        fail("not_found", "这条结果没有下载地址")
    return {"target": {"type": "http", "url": f"{BASE}{path}.{ext}", "ext": ext}}


METHODS = {"info": info, "search": search, "resolve": resolve}


def main():
    method = sys.argv[1] if len(sys.argv) > 1 else ""
    raw = sys.stdin.buffer.read().decode("utf-8").strip()
    req = json.loads(raw) if raw else {}
    if req.get("protocol") != 2:
        sys.stderr.write("这个插件需要协议 2（0.5.0 以上版本的软件）\n")
        sys.exit(2)
    fn = METHODS.get(method)
    if not fn:
        fail("unsupported", f"不支持的方法：{method}")
    try:
        data = fn(req.get("params") or {}, req.get("settings") or {}, req.get("context") or {})
    except Exception as e:  # 没想到的错误也要按约定报告
        fail("internal", f"{type(e).__name__}: {e}")
    emit("result", data=data)


if __name__ == "__main__":
    main()
