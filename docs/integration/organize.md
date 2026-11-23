# 整理文件（organize）集成说明

## 1. 需要在 `lib.rs` 的 `generate_handler!` 里注册的命令

```
organize::org_scan
organize::org_estimate
organize::org_ai_identify
organize::org_execute
organize::org_undo
organize::org_batches
```

`init_modules` 和 `setup_modules` 里已经有 `organize::init` / `organize::setup`，不用再改。

## 2. 依赖

- 不需要新的 Cargo 或 npm 依赖。用到的已有依赖：`trash`（撤销复制 / 硬链接时把副本放进回收站）、`jwalk`、`regex`、`once_cell`、`lofty`（经 `metadata::read_track`）、`tools::probe`（ffprobe，没有时时长按未知处理）。
- 前端用的 `@tauri-apps/plugin-dialog` 已在项目里。

## 3. 需要的前端改动

- 无。`App.svelte` 已经挂载 `Organize`，`ui.svelte.ts` 已经有 `org` 视图。
- 前端文件：`src/lib/organize.ts`、`src/views/Organize.svelte`、`src/components/organize/OrgRow.svelte`、`src/components/organize/OrgHistory.svelte`。

## 4. 命令说明

| 命令 | 参数 | 返回 |
| --- | --- | --- |
| `org_scan` | `roots: string[]`、`types: 'all' \| 'video' \| 'music'`、`target_root: string` | `Item[]` 预览列表（不改动文件） |
| `org_estimate` | `items: Item[]` | `{files, tokens}`：需要交给 AI 的文件数和预计 token（只算提示词） |
| `org_ai_identify` | `items: Item[]`、`target_root: string` | 更新后的 `Item[]`；没有配置 AI 时返回错误 |
| `org_execute` | `items: Item[]`、`mode: 'move' \| 'copy' \| 'hardlink' \| 'library'`、`cleanup_empty?: bool` | `{batch_id: number \| null, done, failed: [{path, error}]}` |
| `org_undo` | `batch_id: number` | `{reverted, failed, finished}` |
| `org_batches` | 无 | 最近 50 批 `{id, created_at, mode, undone, count}` |

与原设计的差异：`org_execute` 返回的不只是 `batch_id`，还带成功数和失败列表；`org_ai_identify` 多了 `target_root`（AI 结果要按模板重算目标位置）；新增 `org_estimate`。

`Item`：`{src, dst, kind, confidence: 'hi'|'mid'|'low', reason, extras: [{src, dst}], checked}`。`dst` 为空字符串表示原地不动。`org_execute` 只处理 `checked` 为 true 且 `dst` 非空的条目；配套文件跟着主文件走。

## 5. 设置（settings 表，键 `org`）

```json
{
  "templates": {
    "anime": "番剧/{作品名} ({年份})/Season {季:02}/{作品名} - S{季:02}E{集:02}.ext",
    "tv": "电视剧/{作品名} ({年份})/Season {季:02}/{作品名} - S{季:02}E{集:02}.ext",
    "movie": "电影/{片名} ({年份})/{片名} ({年份}).ext",
    "music": "音乐/{艺人}/{专辑} ({年份})/{音轨:02} - {歌名}.ext",
    "manga": "漫画/{作品名}/{原文件名}.ext",
    "novel": "小说/{作者}/{书名}.ext",
    "anime_special": "番剧/{作品名} ({年份})/Specials/{作品名} - {特典}.ext",
    "tv_special": "电视剧/{作品名} ({年份})/Specials/{作品名} - {特典}.ext"
  },
  "exclude": ["D:\\某个不要动的文件夹"]
}
```

- 模板用 `/` 分目录；末尾的 `.ext` 换成源文件的扩展名。
- 变量：`{作品名} {片名} {年份} {季} {集} {特典} {艺人} {专辑} {音轨} {歌名} {作者} {书名} {原文件名}`。`{名:02}` 表示不足两位补零。
- 年份未知时，整个 `(…{年份}…)` 括号一起去掉。
- 配置缺失的键用默认模板。

## 6. 识别规则

- 视频：`parse::parse_episode` 拆出作品名、季、集、字幕组、分辨率。有集数时：字幕组命名为“把握高”；番剧 / 电视剧由文件夹名、文件名提示或时长（约 12–32 分钟为番剧，33–75 分钟为电视剧）判断。没有集数、时长 70 分钟以上或名字提示是电影的，按电影处理（把握中）。其余认不出，目标位置留空。
- 特典（NCOP、NCED、OP、ED、SP、OVA、PV、CM 等）放进 `Specials`。
- 音乐：只读标签（艺人、专辑、音轨号）。没有艺人标签的留空。声音指纹没有做。
- 小说：`epub` 读元数据；`txt` 从文件名拆书名和作者。没有作者的留空。
- 漫画：只识别压缩包 / PDF 文件，作品名取所在文件夹名。漫画文件夹（整个文件夹是一部漫画）暂不移动。
- 同一集多个版本：保留分辨率高的（同分辨率取文件大的），其余移到 `<整理到>/重复待处理/`，把握中、默认勾选。
- 配套文件：文件名以视频主名开头、后面紧跟 `.` 的字幕、字体、`.nfo`、`.lrc`、图片，一起移动并改成新主名。同时匹配多个主名时归最长的那个。
- 文件名：替换 Windows 非法字符（全角替代）；保留字（CON、NUL 等）前加 `_`；单段超过 120 字截断，整条路径超过 240 字截断文件名；重名加 ` (2)`、` (3)`。

## 7. 安全规则（实现位置）

- 系统目录（Windows、Program Files、ProgramData、$Recycle.Bin 等，见 `SYSTEM_DIRS`）不作为来源或目标。扫描时还会跳过 `scanner::skip_dir` 的目录（含 AppData）。
- 执行前不动任何文件。硬链接要求源和目标同盘，不同盘直接报错，不做任何移动。
- 每一步先写 `org_moves`（done=0），成功后改成 done=1；失败的那一步删掉记录。只有 done=1 的步骤会被撤销。
- 打不开的文件（多半被占用）跳过并记录在 `failed` 中。
- 同盘移动用 `rename`；跨盘（错误码 17 / 18）先复制、校验大小一致，再删源。删源失败时撤掉副本。
- 复制不覆盖已有文件（`create_new`）。执行时重名会再加序号。
- 成功的移动会更新 `files.path`（保持 id，播放记录和进度保留）。复制和硬链接会把新文件加进媒体库。
- 撤销：倒序移回原位，改回 `files.path`；撤销复制 / 硬链接时，把我们建的副本放进回收站（原文件不动）。整批撤销后 `org_batches.undone` 置为 1。
- 清理空文件夹（可选）：只删被移走文件的父目录，且完全为空、不是目标文件夹或其上级。
- 进度事件：`org://progress`（`{done, total, current, phase}`）；完成后发 `library://changed`。

## 8. 已知问题和未做的部分

- **“只在软件里整理”（library）方式当前只记录方案**：硬盘不动，媒体库里的名字也没改（改显示名需要 db.rs 增加字段，本次不改 db.rs）。界面上有说明。
- “自动整理”（监听下载文件夹）未做。“整理全部”的向导只做成了一页。
- 声音指纹（Chromaprint / AcoustID）未做，音乐只认标签。
- 漫画文件夹（整个文件夹是一部漫画）不移动。
- 目标位置在 scan 时就算好；用户在界面里手改的路径不会检查是否和别的条目冲突，执行时会再加序号避免覆盖。
- 扫描读音乐标签时，`metadata::read_track` 会把封面写进封面缓存目录（和媒体库扫描相同，不写用户文件）。
- 没有 ffprobe 时，视频时长未知，番剧 / 电视剧 / 电影只靠文件名判断，结果多为“把握低”。
- 整个 crate 当前在主仓库里编译不过：`src/fetch.rs` 声明了 `pub mod download;`，但 `src/fetch/download.rs` 还不存在（不是本模块的文件，没有改）。

## 9. 自测

- 单元测试（临时副本里运行，通过）：模板生成（含年份缺失）、非法字符和保留字、长名截断、特典识别、重名序号、配套文件归属（最长主名）、AI 结果写回（AI 最高给“把握中”）、撤销顺序（倒序）、一次真实的移动 + 撤销（临时目录，包含字幕配套文件，校验 `files.path` 与 id 保持、旧文件夹清理、撤销后原位恢复、撤销过的批次不能再撤销）。
- 没有对 `D:\vibe coding\测试\testmedia` 以外的真实文件做任何移动；测试只用临时目录。没有调用 AI 接口。
- `pnpm check`：本模块文件零错误零警告。`pnpm build`：通过。

## 10. 整理文件规则（.em，kind: organize）

详见 `docs/rules/organize.md`。`scan_core` 多了一个参数（生效中的规则，`em::organize::active()` 在扫描线程里取）：每个文件先按规则匹配（`em::organize::match_path`），第一个匹配的生成把握高的条目（`rule_guess` / `rule_dst`，模板用规则自己的或 `template_for`），都不匹配的走原来的 `guess_*`。`load_settings` 改为 `pub(crate)`，规则测试面板用它取模板设置。
