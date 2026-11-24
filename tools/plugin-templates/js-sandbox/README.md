# 脚本模板（js 入口，沙箱）

复制这个文件夹，改 `plugin.json` 的 `id`、`name`、`permissions.hosts`，再改 `main.js` 里的网址和 CSS 选择器。

- 在软件的 QuickJS 沙箱里运行：只能用 `mp.http` 访问 `permissions.hosts` 里的网站，不能读写文件，不能自己下载（没有 `download` 能力），用户不用信任你的程序
- 只有一个入口文件：没有 `import`、`require`、`setTimeout`、`fetch`；要用第三方库先打包成一个文件
- `login: "optional"`：用户同意后，软件会在请求里自动带上他自己在“网站登录”里保存的登录；插件拿不到 Cookie，也不能自己设置 Cookie

检查清单：`python tools/mpplugin_check.py tools/plugin-templates/js-sandbox`（只检查 plugin.json）。
实际运行请在软件里：“设置 → 插件 → 开发者 → 加载插件文件夹…”，然后在插件详情的“测试”页输入关键字。

说明见 [docs/插件开发.md](../../../docs/插件开发.md)。
