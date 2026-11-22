# 自动获取（fetch）集成说明

## 1. 需要在 `lib.rs` 的 `generate_handler!` 里注册的命令

```rust
fetch::fetch_sources,
fetch::fetch_add_source,
fetch::fetch_remove_source,
fetch::fetch_rebuild_rule,
fetch::fetch_search,
fetch::fetch_ai_pick,
fetch::fetch_download,
fetch::fetch_tasks,
fetch::fetch_pause,
fetch::fetch_resume,
fetch::fetch_cancel,
fetch::fetch_retry,
fetch::fetch_get_settings,
fetch::fetch_set_settings,
fetch::fetch_subscribe,
fetch::fetch_unsubscribe,
fetch::fetch_subs,
```

`init` 和 `setup` 已经在 `lib.rs` 的 `init_modules` / `setup_modules` 里调用，不用再改。

前端调用参数用 camelCase（Tauri 默认转换）：`fetch_add_source` 的 `search_template` 对应 `searchTemplate`，`fetch_subscribe` 的 `source_id`、`media_kind` 对应 `sourceId`、`mediaKind`，`fetch_cancel` 的 `delete_files` 对应 `deleteFiles`，`fetch_set_settings` 的参数名是 `value`。

## 2. 依赖

没有新增依赖。用到的已有依赖：`rss`、`scraper`、`librqbit 9`（`default-tls`）、`reqwest`（rustls，`stream`）、`futures-util`、`url`、`regex`、`once_cell`、`parking_lot`、`dirs`、`lofty`、`tokio`。

Tauri 权限：没有新增。只用到 `app.emit`（事件）和 `reqwest`。

## 3. 前端需要的改动

- `App.svelte` 已经 `import Fetch from './views/Fetch.svelte'` 并在 `ui.view === 'get'` 时渲染，不用改。
- `ui.svelte.ts`、`api.ts` 不用改。
- 新增文件：`src/lib/fetch.ts`（类型、命令封装、文案）、`src/components/fetch/Sources.svelte`、`Search.svelte`、`Downloads.svelte`、`FetchSettings.svelte`。
- 设置项保存在 `settings` 表，键名 `fetch`（可以用 `get_setting` / `set_setting` 读写）。

## 4. 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `fetch://progress` | `TaskRow`（完整任务行） | 每个任务的进度、速度、状态变化时推送，进度约每 0.5 秒一次 |
| `fetch://source` | `{ step, status, message, url }` | 添加或重建网页来源时的步骤：打开页面、分析页面结构、生成提取规则、试搜并验证；status 为 run / ok / warn / error |
| `fetch://new` | `{ subId, query, titles }` | 追更发现新集并已加入下载队列 |
| `library://changed` | — | 整理入库后发出（已有事件，不用新增监听） |

## 5. 数据表（`fetch::init` 建表，`IF NOT EXISTS`）

- `fetch_sources(id, kind, name, url, rule_json, created_at, enabled)`：`kind` 为 `rss` / `ytdlp` / `dir` / `web`。网页来源的 `url` 列保存搜索网址模板，`rule_json` 保存 `{mode, template, rules}`。
- `fetch_tasks(id, source_id, title, kind, url, status, progress, speed, size, save_path, error, created_at, finished_at, media_kind)`：`kind` 为 `http` / `magnet` / `torrent` / `ytdlp` / `file`；`status` 为 `queued` / `downloading` / `paused` / `seeding` / `organizing` / `done` / `manual` / `failed` / `canceled`。`manual` 时 `error` 里是“需要手动整理”的说明。
- `fetch_subs(id, source_id, query, media_kind, last_check, seen_json)`。

## 6. 命令清单

| 命令 | 参数 | 返回 |
| --- | --- | --- |
| `fetch_sources` | — | `SourceView[]` |
| `fetch_add_source` | `kind`（rss/ytdlp/dir/web，或 `auto`），`url`，`name?`，`search_template?` | `SourceView`；网页来源会调用 AI 生成规则，过程用 `fetch://source` 推送 |
| `fetch_remove_source` | `id` | — （同时删除该来源的订阅，不删已下载文件） |
| `fetch_rebuild_rule` | `id` | `SourceView`（只对网页来源） |
| `fetch_search` | `query`，`media_kind`（anime/music/video） | `{ hits, groups, notes }` |
| `fetch_ai_pick` | `results`（搜索结果数组），`prefs` | `[{ index, reason }]` |
| `fetch_download` | `items: [{ title, url, kind?, mediaKind?, sourceId?, size? }]` | 新任务 id 数组 |
| `fetch_tasks` | — | `TaskRow[]`（最近 500 条） |
| `fetch_pause` / `fetch_resume` / `fetch_retry` | `id` | — |
| `fetch_cancel` | `id`，`delete_files` | — （删除只限下载目录内的文件） |
| `fetch_get_settings` / `fetch_set_settings` | `value: FetchSettings` | `FetchSettings` |
| `fetch_subscribe` | `source_id`，`query`，`media_kind` | 订阅 id（订阅时把当前结果记为“已见”，只下载之后新出现的集） |
| `fetch_unsubscribe` | `id` | — |
| `fetch_subs` | — | `SubView[]` |

`FetchSettings`：`download_dir`（默认 `下载\极光播放器`）、`organize_dir`（默认 `视频\极光媒体库`）、`concurrency`（1–8，默认 3）、`speed_kbps`（0 为不限，只作用于 HTTP）、`seed_policy`（`stop` 下完即停 / `ratio` 做种到分享率）、`seed_ratio`（默认 1.0）。

## 7. 边界与行为

- 软件不内置任何来源，全部由用户添加。
- 同一站点两次请求之间至少间隔 1 秒（`fetch/source.rs` 的 `throttle`，HTTP、WebDAV、yt-dlp 列表和下载前都会等）。User-Agent 为 `MediaPlayerFetch/1.0`。
- 遇到 401 / 403 或登录页（有密码框）时直接返回“需要登录或验证码”的说明，不重试，也不绕过。
- 下载完成后，可执行文件（exe、bat、cmd、msi、scr、ps1、vbs、js、lnk 等）一律删除。只移动视频和音乐；字幕、图片跟着同名文件移动。认不出的文件留在下载目录，任务标记为 `manual`。
- 整理模板：`番剧\作品名\Season 01\作品名 - S01E03.ext`；`音乐\艺人\专辑 (年份)\03 - 曲名.ext`（需要艺人标签，缺标签交给手动整理）；其他视频 `视频\名字\原文件名`。目标已存在时自动加 ` (2)`。
- 追更每 30 分钟检查一次（启动 60 秒后第一次）。只自动下载带集数的新结果，按网址去重。

## 8. 已做的自测

- `cargo test --lib fetch`：35 个测试全部通过。覆盖：来源类型判断（`guess_kind`）、本地路径判断、RSS 和 Atom 解析（内联 XML）、WebDAV 多状态解析、yt-dlp JSON 与进度行解析、网页精简（`simplify`）、按规则提取和字段检查（内联 HTML）、登录与“无结果”判断、规则存取、可执行文件识别、命名模板、大小文字解析、文件名编码、设置规范化。测试中没有访问网络，也没有下载任何东西。
- `cargo check --lib`：无错误、无警告。
- 项目根 `pnpm check`：我的文件零错误零警告。仅剩 `App.svelte` 对 `Daily.svelte`、`Organize.svelte` 的两条报错（其他模块的文件，不在本模块范围内）。
- `pnpm build`：通过。
- 没有启动程序，没有访问任何网站，没有真的下载。

## 9. 已知问题与未做的部分

- **BT 限速**：`speed_kbps` 目前只作用于 HTTP。librqbit 的限速要在创建会话时设置，本版没有接入。
- **BT 边下边播**：没有做按集顺序的分块策略（librqbit 的默认顺序），番剧整季要等下完或手动排序。
- **BT 暂停**：暂停时从 librqbit 会话里移除（文件保留），继续时重新添加并校验已下部分。程序重启后 BT 任务不会自动恢复，会显示为“已暂停”，需要手动点“继续”。
- **yt-dlp 暂停**：直接结束子进程，继续时由 yt-dlp 自己用 `.part` 续传。
- **网页来源**：只读第一页，没有做翻页（`next` 选择器已解析，但未使用）。学习规则用的示例关键字是 `test`。
- **网页规则自动修复**：0 结果时会重新生成规则（会多消耗一次 AI 额度）；如果页面文字里有“没有找到”“no results”之类的提示，则直接返回空结果，不重新生成。
- **网页 AI 解析**：未在真实网站上验证。登录判断只看是否有密码输入框，验证码页面可能被当作规则失效而重新生成。
- **WebDAV 认证**：只支持把用户名和密码写在地址里（`https://用户:密码@host/...`）。没有接入凭据管理器。
- **yt-dlp 网络请求**：`throttle` 只限制 yt-dlp 启动前的那一次请求，yt-dlp 内部的请求不受限制。
- **整理后的 BT 文件夹**：整理移动完成后会删除变空的子文件夹；非媒体文件（如压缩包）留在下载目录，不会移动，也不会自动删除。
- **ratio 做种**：达到分享率后从会话移除，之后整理移动文件。整理阶段没有用复制代替移动。
- 下载、订阅、整理都没有在真实环境里跑过，需要用户在真机上测试一遍（建议先用一个本地目录来源和一个 HTTP 直链验证）。


## 10. B 站（扫码登录、清晰度、字幕弹幕、番剧归档）

B 站下载不另做下载页：解析出的条目通过 `fetch_download`（`kind='ytdlp'`）进同一个任务队列和下载列表，完成后同样由 `organize::finish` 整理入库。

**文件**：`src-tauri/src/fetch/bili.rs`（登录、账号、解析、yt-dlp 参数）、`src-tauri/src/fetch/ytdlp.rs`（安装 / 更新 yt-dlp）、`src-tauri/src/danmaku.rs`（弹幕 XML ↔ ASS）、`src/components/fetch/Bili.svelte`（面板）、`src/components/media/Danmaku.svelte`（播放器弹幕层）。

**新依赖**：`qrcode 0.14`（只开 `svg`，生成登录二维码）、`sha2 0.10`（校验 yt-dlp）。

| 命令 | 参数 | 返回 |
| --- | --- | --- |
| `fetch_bili_account` | — | `{ logged_in, expired, name, vip, vip_label }` |
| `fetch_bili_qr_start` | — | `{ key, url, svg }` |
| `fetch_bili_qr_poll` | `key` | `waiting` / `scanned` / `expired` / `ok`（ok 时已保存 Cookie） |
| `fetch_bili_logout` | — | 删除本机保存的 Cookie |
| `fetch_bili_parse` | `url` | `{ title, kind: bangumi/video/list, media_kind: anime/video, entries: [{title,url}] }` |
| `fetch_ytdlp_status` | — | `{ installed, version, path, managed }` |
| `fetch_ytdlp_install` | — | 从 yt-dlp 官方发布页下载 `yt-dlp.exe`，按 `SHA2-256SUMS` 校验后放到 `<数据目录>\bin\`；也用于更新 |
| `media_danmaku` | `id`（视频 id） | 视频旁 `<名>.danmaku.ass` 里的弹幕 `[{start,end,mode,y,color,scale,text}]`，没有就是空数组 |

**行为**

- 登录：B 站官方网页扫码接口；Cookie（`SESSDATA`、`bili_jct`、`DedeUserID` 等）存在 Windows 凭据管理器（`keyring`，服务名 `music-player-bili`），不写数据库和日志。每次调用 yt-dlp 时写一份临时 `cookies.txt` 到下载目录，进程结束即删除。
- 设置（`FetchSettings`，都有默认值，旧设置可直接读）：`bili_quality`（画面高度上限，0 = 账号能拿到的最高）、`bili_danmaku`、`bili_subs`。
- yt-dlp 参数（`bili::ytdlp_args`，有单元测试）：B 站加 `--sleep-requests 1`、`-S res[:N],codec:h264`（同清晰度优先 H.264，内置播放器兼容最好）、`--merge-output-format mp4`、`--write-subs --sub-langs all|all,-danmaku|danmaku`；番剧输出名 `作品 - S01E03 [ep…].mp4`，整理时进番剧库；`mediaKind=music`（任何网站）只下音频 `-f ba/b`，有 ffmpeg 时写入标题、UP 主（作艺人）和封面。找到 ffmpeg 时传 `--ffmpeg-location`。
- yt-dlp 任务完成后返回整个 `yt-<id>` 文件夹交给整理，字幕、弹幕跟着同名视频移动。弹幕 XML 先转成 ASS（滚动弹幕按轨道排布，太密时丢弃多余的，高级 / 代码弹幕不转换）。
- `*.danmaku.ass` 不出现在播放器的字幕列表里，由弹幕层单独绘制；按 `D` 或控制栏的“弹”开关，状态记在本机。
- 报错翻译（`bili::friendly_error`）：需要大会员、需要购买、DRM、需要登录、412/429 限流。软件不绕过这些限制。
- `kind_of_url`：B 站网址判为 `ytdlp`。
- 工具查找顺序改为：环境变量 → `<数据目录>\bin\` → 程序目录 `bin\` → PATH。

**自测**：`cargo test --lib`（新增 bili 9 个、danmaku 7 个、ytdlp 2 个测试）、`cargo clippy --all-targets -- -D warnings`、`pnpm check`、`pnpm build` 通过；界面用浏览器加模拟后端数据截图检查了 B 站面板、扫码弹窗、设置和播放器弹幕层。没有连真实 B 站账号测试登录和下载，需要在真机上扫码试一次。
