#!/usr/bin/env python3
"""把 demos-src/core.css 和 core.js 内联进每个 demo，生成可直接双击打开的单文件 HTML。"""
import pathlib
root = pathlib.Path(__file__).resolve().parent.parent
src, out = root / "demos-src", root / "demos"
css, js = (src / "core.css").read_text("utf-8"), (src / "core.js").read_text("utf-8")
pro = (src / "pro.js").read_text("utf-8")
out.mkdir(exist_ok=True)
B = lambda attr, ic, label: f'<button {attr} data-icon="{ic}" title="{label}"><span>{label}</span></button>'
MACROS = {
    "<!--NAV-->": "".join(B(f'data-nav="{k}"', i, l) for k, i, l in [("now", "home", "正在播放"), ("music", "music", "音乐库"), ("video", "video", "视频"), ("anime", "anime", "动漫"), ("novel", "book", "小说"), ("convert", "convert", "格式转换")]),
    "<!--TOOLS-->": "".join(B(f'data-mp="{k}"', i, l) for k, i, l in [("scan-open", "scan", "扫描媒体库"), ("eq-open", "eq", "均衡器"), ("oled-open", "oled", "OLED 防烧屏"), ("immersive", "expand", "沉浸模式"), ("mini", "mini", "迷你播放器"), ("open-file", "folder", "打开本地音乐")]),
    "<!--CTRL-->": '<button data-mp="shuffle" data-icon="shuffle" title="随机"></button><button data-mp="prev" data-icon="prev" title="上一首"></button><button data-mp="play" class="pp"></button><button data-mp="next" data-icon="next" title="下一首"></button><button data-mp="repeat" data-icon="repeat" title="循环"></button>',
    "<!--VIEWS-->": """<section data-view="music" class="lib"><header><h2>音乐库</h2><p>3,862 首 · 412 张专辑 · FLAC / APE / DSD / ALAC / MP3 / Opus 等格式统一管理</p></header><div data-mp="list-music"></div></section>
<section data-view="video" class="lib"><header><h2>视频</h2><p>417 个 · 电影、剧集、纪录片、个人录像 · 硬件解码 · 内嵌与外挂字幕</p></header><div data-mp="grid-video" class="wide"></div></section>
<section data-view="anime" class="lib"><header><h2>动漫</h2><p>296 集 · 按番剧、季、集自动整理，记住每一部看到哪里</p></header><div data-mp="grid-anime"></div></section>
<section data-view="novel" class="lib"><header><h2>小说</h2><p>158 本 · TXT / EPUB / MOBI · 自动识别编码与章节</p></header><div data-mp="shelf-novel"></div></section>
<section data-view="convert" class="lib"><header><h2>格式转换</h2><p>内置 FFmpeg，批量转码、换封装、提取音轨</p></header><div data-mp="convert"></div></section>""",
}
for f in sorted((src / "demos").glob("*.html")):
    html = f.read_text("utf-8")
    if f.name == "index.html":
        (out / f.name).write_text(html, "utf-8"); continue
    assert "/*CORE_CSS*/" in html and "/*CORE_JS*/" in html, f.name
    for k, v in MACROS.items():
        html = html.replace(k, v)
    html = html.replace("/*CORE_CSS*/", css).replace("/*CORE_JS*/", js).replace("/*PRO_JS*/", pro)
    (out / f.name).write_text(html, "utf-8")
    print("built", f.name, len(html) // 1024, "KB")
