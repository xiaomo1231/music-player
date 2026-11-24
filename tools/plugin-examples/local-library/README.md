# 本地曲库（示例插件）

最小的 process 插件：在你自己的曲库文件夹里按关键字搜索，把选中的曲目（和同名 .lrc 歌词）复制进播放器，由播放器补标签、整理入库。不联网，只用 Python 标准库。

- 方法：`info`、`search`、`download`（协议 2，标准输出一行一个事件，带复制进度，支持断点续传）
- 设置：`library_dir`（必填，曲库文件夹）、`max_results`
- 原文件只读不动

文件名按这几种样子识别：`艺人\专辑\03 - 曲名.flac`、`艺人 - 曲名.mp3`、`曲名.m4a`（艺人取上级文件夹名）。

## 本地检查

在项目根目录运行：

```
python tools/mpplugin_check.py tools/plugin-examples/local-library --set library_dir=D:\Music --query 晴天 --download
```

## 在软件里试

“设置 → 插件 → 开发者 → 加载插件文件夹…”选这个文件夹；在插件详情的“概况与设置”里填好曲库文件夹，再到“测试”页输入关键字试一次。装好后它出现在“自动获取 → 我的来源”的“插件”分组里。

## 打包

在开发者模式里点“导出插件包”，或者把这个文件夹里的文件打成 zip、扩展名改为 `.mpx`（`plugin.json` 在 zip 根目录）。

想让没装 Python 的人也能用：`pyinstaller -F main.py` 得到 `main.exe`，在 `plugin.json` 的 `runtime.command` 里加 `"windows": ["main.exe"]`，把 `main.exe` 一起打包。
