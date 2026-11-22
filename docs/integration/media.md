# 视频与番剧（media）集成说明

范围：`src-tauri/src/media.rs`，前端 `views/Videos.svelte`、`views/Anime.svelte`、`overlays/VideoPlayer.svelte`，新增 `components/media/`（Poster、SeriesDetail）和 `lib/media.svelte.ts`。

## 1. 需要在 lib.rs 的 generate_handler! 里注册的命令

`media::init` 和 `media::setup` 已经在 `init_modules` / `setup_modules` 里调用，只差命令注册。在 `// ↓ 各功能模块的命令` 上面加入：

```rust
media::media_probe,
media::media_thumb,
media::media_thumb_dir,
media::media_subtitles,
media::media_sub_vtt,
media::media_save_screenshot,
```

| 命令 | 参数（JS 端用 camelCase） | 返回 |
| --- | --- | --- |
| `media_probe` | `id` | `{duration, width, height, codec, audio:[{index,lang,title,codec}], subs:[{index,lang,title,codec}]}`；同时写回 videos 表的 duration / width / height / codec，并把 `subs` 为 none 而实际有内封字幕的改成 embedded |
| `media_thumb` | `id` | 缩略图文件名（只存文件名）。在 10% 和 25% 处各截一帧，取 JPEG 较大的一张，缩到 480 宽；已存在则直接返回并写回 videos.thumb |
| `media_thumb_dir` | 无 | 缩略图目录的绝对路径（前端用 `fileUrl(目录 + '/' + 文件名)`） |
| `media_subtitles` | `id` | `[{kind:'external', path, label}]` 或 `[{kind:'embedded', stream, label}]`；外挂字幕为同目录下文件名以视频文件名开头的 srt/ass/ssa/vtt（后缀不能紧跟字母数字，避免 01 匹配 010） |
| `media_sub_vtt` | `path?`（外挂字幕文件路径）、`video?`（视频**文件路径**）、`stream?`（内封字幕流的绝对 index） | WebVTT 文本。srt 用纯 Rust 转换（读取时自动识别编码），vtt 原样返回，ass/ssa 及内封流走 `ffmpeg -f webvtt -` |
| `media_save_screenshot` | `videoPath`、`pngBase64`（支持 data URL）、`at?`（播放位置，秒） | 保存的 PNG 路径，文件名 `视频名_01-23-45.png`，同名则加 ` (2)`；放在视频旁边 |

命令实现都是 `async` + `spawn_blocking`，ffmpeg / ffprobe 不会卡住界面。

## 2. 依赖与权限

- 不需要新的 Cargo 依赖（用到的 `blake3`、`base64`、`serde_json`、`rusqlite` 已在 Cargo.toml 中）。
- 需要 ffmpeg.exe / ffprobe.exe（`tools.rs` 的查找顺序：程序目录的 bin、MP_FFMPEG / MP_FFPROBE、PATH）。没有时缩略图、字幕提取会给出错误提示，界面仍可用。
- Tauri 权限已齐：capabilities/default.json 里有 `set-fullscreen`、`is-fullscreen`、`set-always-on-top`；asset 协议 scope 为 `**`，缩略图目录和视频文件都能用 `fileUrl` 访问。

## 3. 其他文件需要的改动（不由本模块修改）

- `App.svelte`：已有 `{#if ui.videoId != null}<VideoPlayer />{/if}`，无需改动。
- `ui.svelte.ts`：本模块只用到已有字段（`videoId`、`videoList`、`seriesId`、`subVideo`、`convertPaths`、`go`、`openItem`）。
- `views/AiSub.svelte`（AI 字幕页）：需要读取 `ui.subVideo`，它是视频的 id（videos.file_id），用来预选要生成字幕的集。
- `views/Convert.svelte`：需要读取 `ui.convertPaths`（播放器“转换为 MP4”会设置它）。
- `api.ts`：`Video` / `Series` 类型已够用；本模块的 `MediaInfo`、`SubItem`、`Cue` 类型放在 `lib/media.svelte.ts`。

## 4. 数据库说明

- 本模块不建表，只读写 `videos` 表的 `duration / width / height / codec / thumb / subs / position / watched / last_played`。
- 进度通过已有命令 `save_video_progress` 保存（每 5 秒和关闭、切集、播完时各一次）。
- 缩略图文件在 `dirs.thumbs`，文件名 = 路径 blake3 前 24 位 + `.jpg`。

## 5. 自测

- `cargo test --lib media`：6 个单元测试全部通过（srt→vtt、缩略图文件名、外挂字幕匹配、ffprobe JSON 解析、时长回退、标签与时间格式）。
- `cargo check`：整个 crate 目前不能编译，原因是 `src-tauri/src/fetch.rs` 里声明了 `download`、`organize`、`web` 三个子模块，但文件还没建（属于其他模块，本模块没有改动）。把这三行去掉后在副本里检查，media.rs 没有任何错误或警告。
- `pnpm check`：本模块的文件 0 错误、0 警告。剩下 7 个错误都在 `App.svelte`，是它引用的 Manga / Novels / Daily / Convert / AiSub / Fetch / Organize / Settings 占位视图没有 `<script lang="ts">`，不是本模块的问题。
- `pnpm build`：通过。

## 6. 已知问题与限制

- 音轨切换、音频延迟（Ctrl+Z / Ctrl+X）需要 mpv 引擎，目前只给出提示。
- 字幕 ASS 样式会丢失（只显示文字）；图形字幕（PGS、VobSub）无法转换，会提示“无法显示”。
- 截图依赖画布读取视频画面。如果 asset 协议返回的资源不允许读取，截图会提示“截图失败”。
- 进度条只显示悬停时间，没有缩略图预览。
- 番剧详情没有“为本季缺字幕的集批量生成 AI 字幕”按钮，只有每集的“生成字幕”。
- 旋转（Ctrl+R）只用 CSS 旋转，不改变容器尺寸。
- “在追 / 未看”的判断用的是 watched 计数和 last_played，不是逐集位置。
