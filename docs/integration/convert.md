# 格式转换（convert）集成说明

涉及文件：`src-tauri/src/convert.rs`、`src/views/Convert.svelte`、`src/lib/convert.ts`、`src/components/convert/Queue.svelte`。

## 1. 需要在 `lib.rs` 注册的命令

`generate_handler!` 里加入（`convert::init` 和 `convert::setup` 已经在 `init_modules` / `setup_modules` 里）：

```rust
convert::convert_presets,
convert::convert_hw,
convert::convert_get_settings,
convert::convert_set_settings,
convert::convert_set_plugin,
convert::convert_add,
convert::convert_list,
convert::convert_cancel,
convert::convert_clear_done,
convert::convert_retry,
convert::convert_undo,
convert::convert_expand,
```

| 命令 | 参数（JS 端用 camelCase） | 返回 |
| --- | --- | --- |
| `convert_presets` | 无 | 预设列表 `{id, name, kind, ext, note}` |
| `convert_hw` | 无 | 硬件编码检测结果 `{checked, h264, hevc, probes}` |
| `convert_get_settings` | 无 | `{out_dir, out_dir_audio, out_dir_video, concurrency, plugin_enabled, plugin_ack, plugin_dir, plugin_file}` |
| `convert_set_settings` | `outDir`（`""` 清除自定义）、`concurrency`（1–4）；不想改的传 `null` | — |
| `convert_set_plugin` | `enabled`、`ack`（开启时必须为 true） | — |
| `convert_add` | `paths`、`preset`、`after`（`lib` / `replace` / `side` / `dir`）、`outDir`（可空） | 新任务 id 列表 |
| `convert_list` | 无 | 最近 200 条任务 |
| `convert_cancel` | `id` | — |
| `convert_clear_done` | 无 | 删除的条数（只删记录，不删文件） |
| `convert_retry` | `id`（只对失败 / 已取消、且没替换过原文件的任务） | — |
| `convert_undo` | `id`（只对“替换原文件”的任务） | — |
| `convert_expand` | `paths`（文件夹展开成媒体文件） | 文件路径列表 |

## 2. 依赖与权限

- Cargo：没有新增依赖。用到的 `trash`、`dirs` 已在 `Cargo.toml` 里。
- npm：`@tauri-apps/plugin-dialog` 已在 `package.json` 里。
- Tauri 权限：`capabilities/default.json` 已有 `dialog:default`，选择文件和文件夹够用。拖放用的是 `core:default` 的事件监听，不需要新增。

## 3. 事件

- `convert://progress`：`{id, progress, speed, status, error, output}`，运行中约每 300ms 一次，状态变化时也会发。
- `convert://hw`：硬件编码检测完成（之前页面显示“正在检测”）。
- `library://changed`：加入媒体库、替换、撤销后发出，媒体库页面据此刷新。

## 4. 与其他模块的衔接（已实现，无需改动）

- 播放栏的“转换当前曲目”和歌曲右键菜单的“转换格式”已经在设置 `ui.convertPaths` 并跳到转换页，转换页挂载时会取走这些路径加入待转换列表。
- 拖放不冲突：`App.svelte` 在 `ui.view === 'convert'` 时直接返回，拖入的文件由转换页自己接收。
- 完成后“播放”会调用 `add_files` 把输出文件加入库，音频交给 `player.playOne`，视频交给 `ui.openItem`。

## 5. 数据约定（jobs 表，没有改表结构）

- 排队 / 运行 / 失败 / 取消时 `jobs.output` 保存“请求的输出目录”；完成后保存实际输出文件。
- 替换原文件时 `jobs.backup` 保存原文件路径（已移入回收站）。撤销时按这个路径从回收站还原。
- 替换过原文件的任务不能直接重试，必须先撤销。

## 6. 完成后的四种处理

- `lib`：新文件放到输出目录，`indexer::index_file` 入库，原文件不动。
- `replace`：先用 `probe` 校验新文件（有流、时长相差不超过 3% 或 2 秒、前 60 秒能解码）→ 原文件移入回收站 → 新文件改名到原位置（扩展名可能变）→ 库里同一行改路径（id 不变，播放次数、收藏、歌单、进度保留）→ 重新读标签。任何一步失败都会回滚：新文件删除，原文件从回收站还原。
- `side`：新文件放在原文件旁边，重名加 ` (1)`，两个都入库。
- `dir`：只输出到目录，不入库。

输出目录的默认值：设置里的 `out_dir`；没有就用“我的音乐\Converted”（音频）或“我的视频\Converted”（视频）。

临时文件写在目标目录的隐藏子文件夹 `.mpc-convert` 里，扫描器会跳过以 `.` 开头的目录，所以半成品不会被当成新文件入库。

## 7. 撤销替换（convert_undo）

1. 确认回收站里有原文件（找不到就直接报错，不动任何东西）。
2. 如果转换出来的文件和原文件同名，先把新文件改名为 `名字 (转换版).扩展名`。
3. 从回收站还原原文件，库里那一行改回原路径，重新读标签。
4. 转换出来的新文件保留在磁盘上，并作为新条目入库，所以不会丢数据。

## 8. 加密格式插件

- 插件是应用数据目录下 `plugins\` 里的第一个 `.exe`，命令行约定 `插件.exe 输入 输出`，输出文件是标准音频。主程序不包含任何解密算法。
- 默认关闭。开启必须先勾选“仅用于本人已购买的内容”，设置项是 `convert_plugin = {enabled, ack}`。
- 未启用或没找到插件时，加密格式的任务直接失败，并给出原因。

## 9. 硬件编码

启动时后台检测 `h264_nvenc / h264_qsv / h264_amf / hevc_nvenc / hevc_qsv / hevc_amf`：先看 `ffmpeg -encoders` 有没有，再用 1 秒测试源真正编一次。H.264 / H.265 预设优先用可用的硬件编码器，否则用 x264 / x265。

## 10. 自测结果

- `cargo test --lib convert`：14 个用例全部通过（进度行解析、编码器列表与选择、预设参数生成、封面映射、输出路径加序号、任务认领、类别切换等）。
- `cargo check`：在项目当前状态下会失败，原因是别人的 `src/fetch.rs` 声明了不存在的子模块（`download` / `organize` / `web`），与本模块无关。我把 crate 复制到临时目录，只把 `fetch.rs` 替换成空壳后检查，没有错误，本模块也没有新警告。
- `pnpm check`：我的三个文件零错误零警告。当前的 5 个错误都在 `App.svelte`，是 `Organize.svelte` 还没提交造成的。
- `pnpm build`：通过。
- 用本机 ffmpeg 手动跑过生成的参数，在 `D:\vibe coding\测试\testout` 输出验证：
  - NVENC H.264 / H.265 可用，QSV 在本机不可用（检测会正确回退）。
  - 压制字幕（烧录同名 srt）、仅换封装、GIF、MP3 封面映射（用 PNG 封面文件）都正常。
- 没有启动程序，所以替换、撤销、拖放和界面交互还没有做端到端测试。

## 11. 已知问题

- 替换前的回收站步骤：如果原文件正在被播放器占用，移入回收站会失败，任务会报错并回滚。
- 崩溃时如果刚好在“移入回收站”之后、“改名”之前，原文件在回收站里，任务会标记为失败并提示“程序退出时中断”，但因为 `backup` 还没记录，不能自动撤销，需要手动到回收站还原。
- 解码校验只检查前 60 秒。
- 烧录字幕用 ffmpeg 默认的字幕编码检测，GBK 编码的 srt 可能显示乱码。
- Opus 输出不保留封面；视频输出（MP4）不带封面。
- “播放界面快捷转换”（原型里播放界面的小弹窗）没有做，只做了转换页和 `ui.convertPaths` 的接入。
- 加密格式插件的输出文件固定叫 `decoded.bin`，依靠内容探测识别格式。
- `dist/` 目录因为 `pnpm build` 被重新生成。
