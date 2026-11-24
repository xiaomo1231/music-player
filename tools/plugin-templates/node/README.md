# Node.js 模板（process 入口）

复制这个文件夹，改 `plugin.json` 的 `id`、`name`、`permissions.hosts`，再改 `main.js` 里的 `search` 和 `resolve`。

- 需要用户电脑上装了 Node.js 18 以上（`node` 在 PATH 里）。不想依赖 Node 时可以用 `pkg` 之类的工具打成单个程序，在 `runtime.command` 里按系统写
- 协议 2：`node main.js <方法>`，标准输入一个请求 JSON，标准输出一行一个事件
- 模板的 `resolve` 演示交给 yt-dlp 下载（`type: "ytdlp"`，只给页面地址和 `format`）

检查：

```
python tools/mpplugin_check.py tools/plugin-templates/node --query 测试
```

说明见 [docs/插件开发.md](../../../docs/插件开发.md)。
