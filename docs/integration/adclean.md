# 小说去广告与阅读器（adclean）集成说明

范围：`src-tauri/src/adclean.rs`、`src/overlays/NovelReader.svelte`、`src/views/Novels.svelte`、`src/overlays/ComicReader.svelte`、`src/views/Manga.svelte`、`src/components/reader/*`、`src/lib/reader.ts`。

## 1. 要注册的命令（lib.rs 的 `generate_handler!`）

以下命令已经写好，需要在 `lib.rs` 的 `tauri::generate_handler![...]` 里加上（本模块不改 lib.rs）：

```
adclean::adclean_scan
adclean::adclean_find_similar
adclean::adclean_estimate
adclean::adclean_ai_rules
adclean::adclean_apply
adclean::adclean_revert
adclean::adclean_list
adclean::adclean_rules
adclean::adclean_rule_set_enabled
adclean::adclean_rule_delete
adclean::adclean_rules_export
adclean::adclean_rules_import
adclean::adclean_export_clean
```

`init` 和 `setup` 已经在 `lib.rs` 的 `init_modules` / `setup_modules` 里调用，不用再加。

## 2. 依赖与权限

- 没有新增 Cargo 依赖（用到的 regex、once_cell、rusqlite、serde_json、anyhow 都已在 Cargo.toml 里）。
- 没有新增 npm 依赖。
- 导出干净副本用的是 `@tauri-apps/plugin-dialog` 的 `save`，`capabilities/default.json` 里已有 `dialog:default`。

## 3. 对其他文件的要求

- `ui.svelte.ts`、`api.ts`、`App.svelte` 不需要改动。
- 阅读器只用到已有命令：`open_book`、`list_books`、`save_book_progress`、`get_setting`、`set_setting`、`list_comics`、`save_comic_progress`、`comic://` 页面（`comicPage`）、`fileUrl`。
- 覆盖层的挂载方式不变（App.svelte 里 `ui.bookId` / `ui.comicId` 非空时显示）。

## 4. 数据约定

### novel_cleanups（db.rs 已有的表，本模块不改表结构）

| 列 | 内容 |
| --- | --- |
| file_id | 书的 files.id |
| chapter | 章号（从 0 开始，与 `open_book` 返回的章节顺序一致） |
| text | JSON：`{"para":段号,"start":段内起点,"end":终点,"text":原文片段,"rule":规则说明}` |
| rule_id | 命中的 novel_rules 行号（手动标记、找相似为空） |
| confidence | `hi` 自动处理 / `mid` 待确认 |
| status | `applied` 已清理 / `pending` 待确认 / `reverted` 已撤销 |

- `start`、`end`、`para` 都是 **Unicode 字符（码点）下标**，左闭右开。前端用 `Array.from(str)` 计算，和 Rust 的 `chars()` 一致。
- 同一处（章、段、起点、终点）只保存一行。

### novel_rules

- `scope`：`global` 或 `book:<file_id>`；`source`：`builtin` / `user` / `ai` / `freq`。
- `kind`：`regex`（正则，作用于归一化后的文本）、`contains`（子串，先归一化再转义）、`line`（正则，整段命中且段落不超过 120 字时整段删除）。
- 内置规则在启动时写入（`source='builtin'`，按 scope+pattern 去重，用户停用后不会被重新打开）。内置规则只能停用，不能删除。
- `freq` 来源的规则只做记录（代表句），用它的 enabled 字段控制对应的频率统计结果是否生效。

### 归一化

全角转半角、去掉空白和常见标点（`，。！？“”（）【】…` 等）、`点`→`.`、`○/〇`→`o`、常见西里尔形近字→拉丁字母、转小写。规则不做归一化（正则里的 `(` `|` 等不会被误删）。

## 5. 命令一览

| 命令 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| adclean_scan | id | Hit[] | 第一层：规则 + 频率统计。hi 记为 applied，mid 记为 pending；已确认或已撤销的同一处不会被改回。会删除旧的 pending 结果重新生成 |
| adclean_find_similar | id, sample | Hit[] | 找相似，**不写库**。status 为 `new` 或已有的状态。样本至少 4 个字 |
| adclean_estimate | id | {tokens, samples, paragraphs} | AI 识别前估算用量（不调用 AI） |
| adclean_ai_rules | id, samples? | RuleRow[] | 调用 AI。samples 为空时本地挑出可疑段落（含前后各一段，最多 300 段）。规则必须能编译、命中可疑段落、且不能铺得太广（超过全书段落的 2%，最少 30 段）才会保存为 `scope=book:<id>` 的 `ai` 规则。调用后需要再跑一次 adclean_scan |
| adclean_apply | id, items: Hit[] | 数量 | 确认清理（有记录则更新为 applied，没有则新建） |
| adclean_revert | id, cleanupIds: number[] | 数量 | 撤销。cleanupIds 为空表示撤销全部已清理的内容；待确认的也可以通过它取消（状态变为 reverted） |
| adclean_list | id | Hit[] | 本书全部记录（包括 pending 和 reverted） |
| adclean_rules | scope? | RuleRow[] | scope 传 `book:<id>` 时同时返回全局规则 |
| adclean_rule_set_enabled | id, enabled | - | 停用或启用一条规则 |
| adclean_rule_delete | id | - | 删除非内置规则 |
| adclean_rules_export | path | 数量 | 导出非内置规则为 JSON |
| adclean_rules_import | path | 新增数量 | 导入规则 JSON，统一保存为全局的用户规则，编译失败的跳过 |
| adclean_export_clean | id, path | 清理的处数 | 导出干净的 TXT 副本（章节标题 + 正文，已清理内容不输出）。不能覆盖原文件 |

`Hit` 字段：`id`、`chapter`、`para`、`start`、`end`、`text`、`rule`、`rule_id`、`confidence`、`status`。

## 6. 阅读器行为

- 打开时读 `list_books` 里的 chapter / offset 续读。`offset` 按**章内滚动比例（0~1）**保存，不是字数。
- 进度保存：翻章、关闭、滚动停止约 1.2 秒后。
- 书签存在 `bookmarks:<file_id>`（set_setting），数组元素 `{chapter, offset, label, at}`。
- 外观（字号、行距、字体、主题）只存 localStorage（`mp.reader`），是每台机器的偏好。
- 键盘：←/→ 或 PgUp/PgDn 翻章；Home/End 章首章尾；B 书签；F 全屏；Ctrl+加/减 字号；Ctrl+0 重置；Ctrl+F 搜索；Esc 先关面板再关阅读器。
- 去广告面板：扫描全书、AI 识别（先显示用量再确认）、待确认逐条确认/取消、已清理逐条撤销、全部撤销、对照原文、导出干净副本。
- 选中文字后出现“标记为广告 / 标记，并找相似的清理全书”。

### 漫画阅读器

- 单页 / 双页 / 长条（1 / 2 / 3）；右→左（默认开启，日漫方向）时 ← 为下一页。
- 点击左右半边翻页（单击稍等 230 毫秒，给双击放大让路）；滚轮翻页（放大后滚轮用于滚动）；Ctrl+滚轮、Ctrl+0 缩放。
- 漫画进度 `comics.page` 是 0 起始的页号。
- 预加载当前页前后 2–3 页。
- PDF（`format='pdf'`）直接用 `<iframe src={fileUrl(path)}>`，不做翻页、进度。
- 读完提示同一系列的下一卷。
- AI 翻译：右侧 `TranslatePanel`，页面上叠 `PageOverlay`。T 键在原文 / 译文之间切换。这两个组件由 mangatr 模块实现，本模块不改它们。

## 7. 自测

- `src-tauri`：`cargo test --lib adclean` 通过 9 项（归一化、变形网址识别、句中删除只删中间、整段水印、QQ 群、章尾水印的频率统计、找相似、句号后接英文不误判、短样本拒绝）。`cargo check --lib --tests` 对 adclean.rs 没有警告。
- 项目根目录：`pnpm build` 通过。`pnpm check` 对本模块的文件零错误、零警告。`App.svelte` 里 Daily / Organize 的两条错误来自其他模块，不是本模块引起的。
- 未启动程序，未做界面实测。

## 8. 已知问题与后续

- 没有做“替换原文件”（设计里提到的可选项）。目前只导出副本。
- 设计里的“断行修复、多余空行、重复章节、章节标题规范化”没有做。
- 频率统计按章首 2 段、章尾 2 段的句子做近似合并（Dice ≥ 0.8）。章节数少于 3 章时不做频率统计。
- AI 识别只取本地挑出的可疑段落，没有“精读模式”（逐章检查）。
- 内置规则写入后不会因为版本升级而自动更新（已存在则跳过）。需要改内置规则时，要么改 scope/pattern 组合，要么在 init 里加迁移。
- 找相似依赖“连续覆盖”，对句中插入两处以上的不同词、或换序的变体识别率会下降。

## 9. 小说排版规则（.em，kind: clean）

详见 `docs/rules/clean.md`。和本模块相关的改动：

- `novel_rules.source` 新增 `em:<规则 id>`：规则文件的 `remove` 同步过来的规则。把握按 `hi` 算；`adclean_rule_set_enabled` / `adclean_rule_delete` 拒绝修改它们（提示到“设置 › 规则文件”里管理）；`adclean_rules_export` 不导出它们；`RulesList.svelte` 里显示为只读，同一条按书限定的规则只显示一次。
- 打开书统一走 `em::clean::open_book`（阅读器的 `open_book` 命令和本模块的 `book_of`），套用生效的排版规则；清理记录的下标以套用之后的文字为准。
- `relocate_cleanups`：每次打开书时检查清理记录，对不上当前文字的按原文片段重新定位（同样的文字取离原位置最近的一处）。找不到的保留原样，阅读器（`hitMatches`）和导出都会跳过。
- `adclean_export_clean` 先删广告再按规则的 `indent` / `blank_lines` 排版（`export_text_with`），并跳过对不上的记录。
- `open_book` 命令的返回多了 `layout` 字段（`{indent, blank_lines, rules}` 或 null），阅读器据此设置段首缩进和段间距。
