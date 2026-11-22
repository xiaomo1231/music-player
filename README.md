# music-player

Windows 上的综合媒体播放 / 管理软件：音乐、视频、动漫、小说，内置格式转换。基于 Tauri 2 + Rust。

正式版代码已开工（`src/` 前端、`src-tauri/` Rust 核心），开发方法见 [开发说明](docs/开发说明.md) 和 [开发约定](docs/开发约定.md)，协作者请先读 [参与开发](CONTRIBUTING.md)；发布新版本和自动更新见 [发布流程](docs/发布流程.md)。

方案与原型：

- [技术方案](docs/技术方案.md)
- [界面原型](demos/index.html)：单文件 HTML，下载后双击即可打开，无需联网或安装。界面风格定为“极光玻璃”

## 修改原型

原型的页面骨架和样式在 `demos-src/demos/index.html`，基础逻辑（播放模拟、歌词、扫描动画、视频播放器、OLED 防烧屏等）在 `demos-src/core.js` 和 `core.css`，各功能模块（音乐库与歌单、番剧、漫画翻译、AI 字幕、转换、自动获取）在 `demos-src/pro.js`。改完后运行：

```
python tools/build_demos.py
```

会把公共部分内联进去，重新生成 `demos/` 下的单文件 HTML。

原型里的曲目、歌词、片名、书名均为虚构；声音由浏览器实时合成，不包含任何音频文件。
