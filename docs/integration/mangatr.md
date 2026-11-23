# 漫画 AI 翻译嵌字（mangatr）集成说明

对应技术方案第 15 节。第一版不引入本地模型：检测 + OCR 用图像识别模型一次完成，翻译用文本模型，擦除用取样底色盖住原文。

## 1. 需要在 `lib.rs` 的 `generate_handler!` 里注册的命令

```
mangatr::mangatr_get,
mangatr::mangatr_translate_page,
mangatr::mangatr_translate_range,
mangatr::mangatr_cancel,
mangatr::mangatr_estimate,
mangatr::mangatr_update_block,
mangatr::mangatr_retranslate_block,
mangatr::mangatr_glossary_set,
mangatr::mangatr_export,
```

前端调用时参数用 camelCase（Tauri 自动转换）：`comicId`、`page`、`from`、`to`、`blockId`、`tr`、`glossary`、`applyAll`、`pagesPng`。

## 2. 依赖与权限

- 新依赖：无。用到的 `image`（jpeg 特性已开）、`zip`、`base64`、`once_cell`、`parking_lot` 都已在 `Cargo.toml` 里。
- Tauri 权限：无新增。事件用 `app.emit` 推送，前端 `listen` 用默认事件权限。
- 数据：`<应用数据目录>/mangatr/<comic_id>.json`，结构 `{pages:{"<页号>":{blocks:[…]}}, glossary:[{src,tr}]}`。页号从 0 开始，与 `comicPage(id, page)` 一致。损坏时改名为 `.json.bak` 再报错，不覆盖。

## 3. 前端需要的接入

- 阅读器右侧栏：`<TranslatePanel comicId page pages mode onmode />`（`src/components/mangatr/TranslatePanel.svelte`）。外层已是 `class="rside"`，放在 `.rbody` 的第二列即可。
- 页面图片上：`<PageOverlay comicId page mode />`。父元素需要 `position: relative`，图片铺满父元素。`mode='orig'` 时什么都不显示。
- 两个组件共用 `src/lib/mangatr.svelte.ts` 里的 `mangatr` 状态（当前书、选中句子、保存、进度）。
- 不需要改 `ui.svelte.ts`、`api.ts`、`App.svelte`。面板用到 `ui.open('ai')`（已有的 AI 服务弹窗）。
- 进度事件 `mangatr://progress`，载荷 `{comic_id, page, from, to, done, total, stage, message}`。`stage` 为 `load` / `detect` / `translate` / `saved`，整话结束时为 `done` / `error` / `cancelled`。组件内部已监听，阅读器不需要处理。
- 导出用的 `renderPage(comicId, page)` 也在 `mangatr.svelte.ts` 里，供其他入口调用。

## 4. 命令一览

| 命令 | 参数 | 返回 | 说明 |
| --- | --- | --- | --- |
| `mangatr_get` | comic_id | `BookTr` | 全书译文和术语表 |
| `mangatr_translate_page` | comic_id, page | `PageTr` | 识别 + 翻译一页，成功才保存 |
| `mangatr_translate_range` | comic_id, from, to | — | 后台逐页翻译（含两端），进度走事件；同一本书同时只能有一个任务 |
| `mangatr_cancel` | comic_id | — | 取消后台任务（在下一步前生效） |
| `mangatr_estimate` | comic_id, from, to | `{pages, per_page, total, note}` | 预计 token，粗略估算 |
| `mangatr_update_block` | comic_id, page, block_id, tr | `Block` | 手动改译文，立即保存 |
| `mangatr_retranslate_block` | comic_id, page, block_id | `Block` | 带同页和前后页上下文重译一句，给出备选 |
| `mangatr_glossary_set` | comic_id, glossary, apply_all | 替换次数 | 改术语表；apply_all 时把已有译文里的旧译法换成新译法 |
| `mangatr_export` | comic_id, pages_png | 新文件路径 | 输出 `原文件名 [中文].cbz` 到原文件旁边。有译文的页用传入的 PNG，其余页拷原图 |

`mangatr_cancel` 是 `mangatr` 模块自己的命令，前缀与其他命令一致。

## 5. 行为说明

- 识别：整页缩到长边 ≤ 1600px 的 JPEG 发给图像识别模型，返回每个文字块的 bbox（0–1）、是否竖排、阅读顺序、原文、类型。`sfx`（拟声词）和 `art`（画面文字）标记为 `skipped`，不翻译。
- 翻译：整页（连同前后页已识别的原文作上下文）加术语表，一次交给文本模型，要求输出与输入条数相同的数组。条数不对或格式不对时重试一次，仍失败则报错，不保存。
- 底色：对文字框外圈取中位数颜色作为 `bg`。前端用它盖住原文，字色按底色亮度自动选黑或白。
- 排版：字号从大到小试，直到能放进气泡（留 6% 边距）。横排按词换行；竖排按列，列从右到左。`PageOverlay` 和导出用的是同一套 `layout.ts`。
- 导出文件名：同名已存在时加时间后缀（`原文件名 [中文] 20261121-010101.cbz`），不覆盖旧版。
- PDF 来源暂不支持翻译和导出，返回明确提示。RAR / CBR 不支持（只读 ZIP / CBZ 和图片文件夹）。

## 6. 自测结果

- `cargo test --lib mangatr`：11 个测试全部通过。覆盖 bbox 归一化与裁剪（含百分比、非法值）、识别结果解析容错（字符串数字、对象 bbox、缺字段、空文字）、无序时的阅读顺序、译文条数对齐、术语替换与全书应用、导出文件名与时间戳、底色取样、提示词拼装、数据键格式。
- `cargo check`：零警告。
- `pnpm check`：我的文件（`src/lib/mangatr.svelte.ts`、`src/components/mangatr/*`）零错误零警告。整体仍有 7 个错误，都在 `App.svelte` 里引用的 `views/Manga.svelte`、`Novels.svelte` 等尚未存在的视图文件，不是这里造成的。
- `pnpm build`：通过。
- 没有调用任何 AI 接口，也没有启动程序。端到端（真实模型识别、翻译、嵌字效果）未验证。

## 7. 已知问题

- 未做端到端验证：识别框的准确度、竖排文字的视觉效果、底色取样效果都依赖真实模型和真实漫画，需要在有 AI 密钥时实测。
- 前端没有单元测试：项目没有 vitest。排版和取样的纯函数在 `layout.ts`，可以之后补测试。
- DOM 叠加层和导出的 canvas 竖排排版是近似一致，字间距可能略有差异。
- 重译或重新翻译整页会覆盖手动改过的译文。
- 底色只取文字框外圈的中位数。气泡边缘有渐变或花纹时可能盖得不干净。
- 拟声词和画面文字第一版不翻译，也不擦除，保留原样。
- 用量估算按每页约 1600px 图片粗估，实际以服务商计费为准。
- 只有成功翻译的页才写入数据文件。中途失败的整页识别结果会丢弃，重试会再次识别（会再次消耗额度）。
- 后台整话翻译没有断点续传。应用退出后需要重新开始，已完成的页会保留。

## 本地文字识别（localocr.rs、textdet.rs）

- 命令：`localocr::localocr_status`、`localocr_download`、`localocr_cancel`、`localocr_remove`、`localocr_test`；进度事件 `localocr://progress`。
- 设置：`AiConfig.ocr_engine` 为 `local` 时，`mangatr::run_page` 改走 `detect_local`：`textdet::detect` 找气泡 → `textdet::crop_png` 裁图放大 → `localocr::recognize` 识别。翻译仍走云端文本模型。
- 依赖：`tract-onnx`（检测模型推理）、Windows 下 `windows-sys`（作业对象，主程序退出时结束 llama-server）。
- 随包文件：`bin\llama\`（llama-server 及其 dll）、`bin\ocr\ch_PP-OCRv4_det_infer.onnx`，见 `tools/bundled-tools.json`。
