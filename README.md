# music-player

Windows 上的综合媒体播放 / 管理软件：音乐、视频、动漫、小说，内置格式转换。基于 Tauri 2 + Rust。

目前处于方案与界面设计阶段：

- [技术方案](docs/技术方案.md)
- [10 款界面原型](demos/index.html)：单文件 HTML，下载后双击即可打开，无需联网或安装

## 修改原型

每个 demo 的源文件在 `demos-src/demos/`，公共逻辑（播放模拟、歌词、扫描动画、OLED 防烧屏等）在 `demos-src/core.js` 和 `core.css`。改完后运行：

```
python tools/build_demos.py
```

会把公共部分内联进去，重新生成 `demos/` 下的单文件 HTML。

原型里的曲目、歌词、片名、书名均为虚构；声音由浏览器实时合成，不包含任何音频文件。
