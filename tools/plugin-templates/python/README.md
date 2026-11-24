# Python 模板（process 入口）

复制这个文件夹，改 `plugin.json` 的 `id`、`name`、`permissions.hosts`，再改 `main.py` 里的 `search` 和 `resolve`。

- 协议 2：`python main.py <方法>`，标准输入一个请求 JSON，标准输出一行一个事件
- 方法：`info`（自检）、`search`、`resolve`；要分集加 `episodes`，要自己下载加 `download`（同时在 `capabilities` 里声明）
- 模板不联网，`search` 返回假数据；`http_get()` 是真联网时用的写法

检查：

```
python tools/mpplugin_check.py tools/plugin-templates/python --query 测试
```

说明见 [docs/插件开发.md](../../../docs/插件开发.md)。
