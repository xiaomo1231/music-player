# AI 字幕 / AI 服务（subtitle）

负责人：字幕模块。涉及文件：`src-tauri/src/subtitle.rs`、`src/views/AiSub.svelte`、`src/components/AiModal.svelte`、`src/lib/aisub.ts`。

## 1. 需要在 `lib.rs` 的 `generate_handler!` 里注册的命令

```rust
subtitle::sub_list_videos,
subtitle::sub_start,
subtitle::sub_batch,
subtitle::sub_cancel,
subtitle::sub_status,
subtitle::sub_result,
subtitle::sub_save,
subtitle::sub_estimate,
subtitle::sub_realign,
```

`ai::ai_get_config`、`ai::ai_save_config`、`ai::ai_test` 已经注册。`subtitle::init` / `subtitle::setup` 已在 `lib.rs` 里调用，无需改动。

| 命令 | 参数（JS 侧用 camelCase） | 返回 | 说明 |
| --- | --- | --- | --- |
| `sub_list_videos` | 无 | `SubVideo[]` | 所有视频：独立视频 + 番剧 / 剧集（kind = anime、tv）的每一集，带 `subs` 状态 |
| `sub_start` | `videoId`, `options` | `u64` 任务 id | 排进队列，后台执行 |
| `sub_batch` | `seriesId`, `season`（可为 null，表示全部季）, `options` | `u64[]` | 只排 `subs = 'none'` 的集 |
| `sub_cancel` | `job` | — | 排队中的直接移除；正在跑的在下一步开始前停止（一次识别请求不能中途打断） |
| `sub_status` | 无 | `JobView[]` | 最近 50 个任务的状态 |
| `sub_result` | `job` | `Line[]` | 分段结果 `{start, end, src, zh}` |
| `sub_save` | `job`, `segments` | `string[]` 输出路径 | 用户改过后重新写出文件，并把 `videos.subs` 设为 `ai` |
| `sub_estimate` | `videoId` | `{minutes, est_tokens, monthly_limit, used_tokens, over_limit}` | 开始前估算；按时长估算的上限，不是精确值 |
| `sub_realign` | `videoId`, `subPath` | `string` 新文件路径 | 只做对齐：用人声区间整体偏移并贴齐已有 srt，写成 `同名.aligned.srt` |

`options`（`SubOptions`）：`lang`（`auto` / `ja` / `en` / `zh`，可为空）、`target`（`both` 双语中文在上 / `zh` / `src` 仅原文）、`output`（`srt` / `ass` / `mkv`）、`glossary`（可为空；有内容时存进 settings，之后同一剧集共用）。

进度事件：`sub://progress`，负载 `{job, step, progress, message}`，`step` 为 `extract` / `vad` / `asr` / `align` / `translate` / `done` / `error`。

## 2. 依赖

- Cargo：无新增依赖。使用已有的 `once_cell`、`parking_lot`、`serde_json`、`anyhow`、`tauri`。
- 外部程序：`ffmpeg.exe`（提取音轨、silencedetect、切块、mkv 封装），需要包含 `libmp3lame`（切块用 mp3 64k）。`ffprobe.exe` 用来读时长。
- npm：无新增依赖。
- Tauri 权限：无新增（使用的是应用自己的命令和事件）。

## 3. 前端需要的东西（已完成，无需集成方改动）

- `AiSub.svelte` 读取 `ui.subVideo`（若非空则默认选中并清空），“去设置”按钮调用 `ui.open('ai')`，“在播放器中预览”调用 `ui.openItem(id, 'video')`。
- `AiModal.svelte` 使用 `ai_get_config` / `ai_save_config` / `ai_test`。弹窗关闭后 `AiSub` 会重新读取 AI 状态。
- `App.svelte` 已有 `AiSub` 和 `AiModal` 的挂载，不需要修改。

## 4. 处理流程（subtitle.rs）

1. `extract`：`ffmpeg -vn -ac 1 -ar 16000 -c:a flac` 到 `缓存目录/subs/<视频 id>/audio.flac`，任务结束后删除。
2. `vad`：在音频上跑 `silencedetect=noise=-35dB:d=0.4`，静音的补集为人声；间隔 0.8 秒以内的区间合并；少于 0.3 秒的丢掉。
3. 切块：每块不超过 60 秒，且估算字节不超过 20 MB（mp3 64k 约 8 KB/s）。超长的连续人声先切开，块两端各留 0.3 秒。
4. `asr`：每块用 `ffmpeg` 导出 mp3 64k，调用 `ai::transcribe`，时间戳加上块的起点。
5. `align`：按句末标点切句，时间按字数比例分配；没有停顿（时间差 > 0.7 秒或中间有静音）的短句合并；中日文每条不超过 40 字（2 行 × 20），英文 84 字符；时长 1–7 秒；起止时间贴到最近的人声边界（0.5 秒以内）。
6. `translate`（target 不是 `src` 时）：每批 40 句，附前 3 句和后 2 句作为语境（只用于理解，不翻译）和术语表。要求返回 `{"lines": [...]}`，条数必须一致，不一致重试一次。
7. `done`：写出 `视频名.zh.srt` / `视频名.zh.ass`（仅原文时为 `视频名.src.*`），或用 `ffmpeg -c copy` 封装成 `视频名.subbed.mkv`。随后把 `videos.subs` 设为 `ai`。

术语表在 settings 里的键：`glossary:s<series_id>`（番剧 / 剧集，整季共用）或 `glossary:v<video_id>`（独立视频）。与任务说明里写的 `glossary:<series_id 或 video_id>` 略有不同，是为了避免番剧 id 和视频 id 撞车。

## 5. 已做的自测

- `subtitle.rs` 的 15 个单元测试全部通过。因为 `src-tauri` 当前有别人正在开发的 `fetch.rs`（缺少 `download` / `organize` / `source` / `web` 子模块），整个 crate 暂时无法编译，所以在独立的临时 crate 里用桩（`ai`、`library`、`tools`、`AppState`）编译 `subtitle.rs` 后测试。测试覆盖：静音解析、人声补集、区间合并、切块（时长和字节两种上限）、句末切分、均匀切分、断句对齐（时长、字数、不重叠）、贴边界、srt 往返、ass 样式、已有字幕对齐、翻译条数校验、时间格式。
- 用本机 ffmpeg 生成带静音的测试音频，确认 `silencedetect` 输出的 `silence_start:` / `silence_end: … | silence_duration:` 格式与解析一致。
- 没有调用任何 AI 接口，没有启动程序。

## 6. 已知问题 / 待办

- 只做云端 API（OpenAI 兼容接口）。原型里的“本地模型”（whisper.cpp）没有实现，`AiSub` 界面上也没有引擎选择。
- 取消：正在进行的一次识别请求无法中途打断，要等这一块返回后才停止。
- 时间轴只画字幕块，没有显示人声区间（后端没有把 VAD 区间传给界面）。
- `sub_estimate` 的翻译用量是按时长的粗略估算（每分钟约 600 tokens），语音识别按秒计费，不计入 token。
- `sub_realign` 只提供命令，界面上没有入口（需要文件选择框，留给集成时决定）。
- `sub_save` 只在任务还留在内存里时可用（应用重启后旧任务的分段结果不再可用，需要重新生成）。
- 整季批量时，所有集共用同一个术语表，但每集的翻译上下文只在该集内部。
- 没有做 `cargo test` 在完整 crate 下的运行，等 `fetch.rs` 修好后应当再跑一次 `cargo test --lib subtitle` 和 `cargo check`。
