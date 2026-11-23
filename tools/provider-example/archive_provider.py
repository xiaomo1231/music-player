"""外部程序下载源示例：Internet Archive 上 CC / 公有领域授权的音频。

协议见 docs/下载源接口.md 第 9 节。只用 Python 标准库，可以直接用，也可以打包成 exe：
    pip install pyinstaller
    pyinstaller --onefile --name archive_provider archive_provider.py
然后在“自动获取 → 我的来源 → 选择程序…”里选 dist\\archive_provider.exe。
没打包时可以写一个同目录的 archive_provider.cmd：
    @python "%~dp0archive_provider.py" %*
"""
import json
import sys
import urllib.parse
import urllib.request

UA = "MediaPlayerFetch/1.0 (external provider example)"
# 只搜明确标了 CC 或公有领域授权的条目
LICENSE = '(licenseurl:*creativecommons.org* OR licenseurl:*publicdomain*)'


def get_json(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=20) as r:
        return json.load(r)


def info(_req):
    return {
        "id": "archive-audio",
        "name": "Internet Archive 音频",
        "media_kinds": ["music"],
        "note": "Internet Archive 上 CC / 公有领域授权的音频（示例程序）",
    }


def search(req):
    q = (req.get("query") or "").strip()
    if not q:
        return []
    page = int(req.get("page") or 1)
    params = urllib.parse.urlencode(
        [
            ("q", f"({q}) AND mediatype:audio AND {LICENSE}"),
            ("fl[]", "identifier"),
            ("fl[]", "title"),
            ("fl[]", "creator"),
            ("fl[]", "year"),
            ("rows", "30"),
            ("page", str(page)),
            ("output", "json"),
        ]
    )
    docs = get_json("https://archive.org/advancedsearch.php?" + params)["response"]["docs"]
    items = []
    for d in docs:
        creator = d.get("creator") or ""
        if isinstance(creator, list):
            creator = creator[0] if creator else ""
        title = d.get("title") or d["identifier"]
        items.append(
            {
                "id": d["identifier"],
                "title": title,
                "artist": creator or "Internet Archive",
                "album": title,
                "year": int(d["year"]) if str(d.get("year", "")).isdigit() else None,
                "cover": f"https://archive.org/services/img/{d['identifier']}",
                "media_kind": "music",
                "qualities": [
                    {"key": "flac", "label": "FLAC", "format": "flac"},
                    {"key": "mp3", "label": "MP3", "format": "mp3"},
                ],
            }
        )
    return items


def resolve(req):
    item = req["item"]
    want = req.get("quality") or "flac"
    meta = get_json("https://archive.org/metadata/" + urllib.parse.quote(item["id"]))
    files = [f for f in meta.get("files", []) if f.get("name", "").lower().endswith((".flac", ".mp3", ".ogg"))]
    if not files:
        return {"error": "这个条目里没有音频文件"}
    files.sort(key=lambda f: (not f["name"].lower().endswith("." + want), f["name"]))
    name = files[0]["name"]
    ext = name.rsplit(".", 1)[-1].lower()
    url = f"https://archive.org/download/{urllib.parse.quote(item['id'])}/{urllib.parse.quote(name)}"
    return {"target": {"type": "http", "url": url, "ext": ext}, "lyric": None}


def main():
    action = sys.argv[1] if len(sys.argv) > 1 else ""
    raw = sys.stdin.buffer.read().decode("utf-8") or "null"
    req = json.loads(raw) or {}
    handlers = {"info": info, "search": search, "resolve": resolve}
    if action not in handlers:
        print(f"未知动作：{action}", file=sys.stderr)
        sys.exit(2)
    try:
        out = handlers[action](req)
    except Exception as e:  # 最后一行 stderr 会显示给用户
        print(f"Internet Archive 请求失败：{e}", file=sys.stderr)
        sys.exit(1)
    sys.stdout.buffer.write(json.dumps(out, ensure_ascii=False).encode("utf-8"))


if __name__ == "__main__":
    main()
