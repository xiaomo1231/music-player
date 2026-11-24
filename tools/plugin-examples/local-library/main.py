"""本地曲库示例插件（process 入口，协议 2）。

只用 Python 标准库。软件运行 `python main.py <方法>`，把请求 JSON 写进标准输入，
从标准输出按行读事件（log / progress / result / error）。

文件名按这几种样子识别：
    艺人\\专辑\\03 - 曲名.flac
    艺人 - 曲名.mp3
    曲名.m4a           （艺人取上级文件夹名）
"""
import json
import os
import re
import shutil
import sys

AUDIO = {".mp3", ".flac", ".m4a", ".aac", ".ogg", ".opus", ".wav", ".ape", ".wma"}
CHUNK = 1 << 20


def emit(event, **kw):
    kw["event"] = event
    sys.stdout.buffer.write((json.dumps(kw, ensure_ascii=False) + "\n").encode("utf-8"))
    sys.stdout.flush()


def fail(code, message):
    emit("error", code=code, message=message)
    sys.exit(0)


def library_dir(settings):
    d = (settings or {}).get("library_dir") or ""
    if not d:
        fail("bad_config", "先在插件设置里选择曲库文件夹")
    if not os.path.isdir(d):
        fail("bad_config", f"找不到曲库文件夹：{d}")
    return os.path.abspath(d)


def describe(root, path):
    rel = os.path.relpath(path, root)
    parts = rel.split(os.sep)
    stem, ext = os.path.splitext(parts[-1])
    title, artist, album, track = stem, "", "", None
    m = re.match(r"^(\d{1,3})\s*[-.]\s*(.+)$", stem)
    if m:
        track, title = int(m.group(1)), m.group(2)
    m = re.match(r"^(.+?)\s+-\s+(.+)$", title)
    if m:
        artist, title = m.group(1), m.group(2)
    if len(parts) >= 3:
        artist = artist or parts[-3]
        album = parts[-2]
    elif len(parts) == 2:
        artist = artist or parts[-2]
    return {
        "id": rel.replace(os.sep, "/"),
        "title": title.strip(),
        "artist": artist.strip() or "未知艺人",
        "album": album.strip(),
        "track": track,
        "media_kind": "music",
        "qualities": [{"key": "orig", "label": ext[1:].upper(), "format": ext[1:].lower(),
                       "size": os.path.getsize(path)}],
    }


def search(params, settings):
    root = library_dir(settings)
    words = [w for w in (params.get("query") or "").lower().split() if w]
    if not words or int(params.get("page") or 1) > 1:
        # 一次就把结果都给出来了，没有第 2 页
        return []
    limit = int((settings or {}).get("max_results") or 50)
    out = []
    for dirpath, _dirs, files in os.walk(root):
        for f in sorted(files):
            if os.path.splitext(f)[1].lower() not in AUDIO:
                continue
            path = os.path.join(dirpath, f)
            hay = os.path.relpath(path, root).lower()
            if all(w in hay for w in words):
                out.append(describe(root, path))
                if len(out) >= limit:
                    emit("log", level="info", msg=f"超过 {limit} 条，只显示前 {limit} 条")
                    return out
    return out


def safe_source(root, item_id):
    path = os.path.abspath(os.path.join(root, *item_id.split("/")))
    if os.path.commonpath([root, path]) != root:
        fail("bad_config", "条目不在曲库文件夹里")
    if not os.path.isfile(path):
        fail("not_found", f"文件已经不在了：{item_id}")
    return path


def copy_with_progress(src, dst, resume):
    total = os.path.getsize(src)
    part = dst + ".part"
    done = os.path.getsize(part) if resume and os.path.exists(part) else 0
    with open(src, "rb") as fi, open(part, "ab" if done else "wb") as fo:
        fi.seek(done)
        while True:
            buf = fi.read(CHUNK)
            if not buf:
                break
            fo.write(buf)
            done += len(buf)
            emit("progress", done=done, total=total, unit="bytes")
    os.replace(part, dst)


def download(params, settings):
    root = library_dir(settings)
    item = params.get("item") or {}
    dest = params.get("dest_dir") or ""
    if not dest or not os.path.isdir(dest):
        fail("internal", "dest_dir 不存在")
    src = safe_source(root, item.get("id", ""))
    dst = os.path.join(dest, os.path.basename(src))
    copy_with_progress(src, dst, bool(params.get("resume")))
    files = [{"path": dst, "role": "main"}]
    lrc = os.path.splitext(src)[0] + ".lrc"
    if os.path.isfile(lrc):
        lrc_dst = os.path.join(dest, os.path.basename(lrc))
        shutil.copyfile(lrc, lrc_dst)
        files.append({"path": lrc_dst, "role": "lyric"})
    return {"files": files}


def main():
    method = sys.argv[1] if len(sys.argv) > 1 else ""
    raw = sys.stdin.buffer.read().decode("utf-8").strip()
    req = json.loads(raw) if raw else None
    if not (isinstance(req, dict) and req.get("protocol") == 2):
        # 协议 1 的软件会直接把 params 写进来；这个示例只支持协议 2
        sys.stderr.write("这个插件需要 0.5.0 以上版本的软件（协议 2）\n")
        sys.exit(2)
    params, settings = req.get("params") or {}, req.get("settings") or {}
    if method == "info":
        result = {"ok": True, "message": f"曲库：{library_dir(settings)}"}
    elif method == "search":
        result = search(params, settings)
    elif method == "download":
        result = download(params, settings)
    else:
        fail("unsupported", f"不支持的方法：{method}")
    emit("result", data=result)


if __name__ == "__main__":
    main()
