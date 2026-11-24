# 规则模板（rule 入口，.em）

把一个 `kind: source` 的 .em 规则包装成插件，好处是可以带说明、图标，打成 `.mpx` 分发，从仓库安装和更新。

- `runtime.file` 指向 .em 文件；装好后注册成下载源 `em-<规则 id>`（这里是 `em-my-em-source`）
- `plugin.json` 的 `permissions.hosts` 必须和 .em 里的 `hosts` 完全一样
- 只能有一个下载源；能力写 `search` + `resolve`（要分集就在 .em 里写 `episodes` 阶段，并在 capabilities 里加上）
- .em 的写法见 [docs/rules/source.md](../../../docs/rules/source.md)

开发时不用打包：直接在“设置 → 规则文件 → 导入…”里导入 `source.em`，改完保存就生效（开发者模式不支持 rule 入口）。写好后先删掉单独导入的这条规则（同一个规则 id 不能既是单独导入的又属于插件），再连同 `plugin.json` 打成 `.mpx` 安装。

检查清单：`python tools/mpplugin_check.py tools/plugin-templates/em`。

说明见 [docs/插件开发.md](../../../docs/插件开发.md)。
