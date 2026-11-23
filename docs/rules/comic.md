# 漫画排版规则（.em，kind: comic）

一份漫画排版规则告诉软件：哪些漫画用这套排版、打开时怎么翻页、页面要不要拆跨页 / 裁白边 / 跳过某些页，以及“导出新 CBZ”时怎么转换。规则不联网，只处理本地的 CBZ / ZIP 和图片文件夹（PDF 不处理）。

- **阅读时**：页面在打开时实时处理，原文件不动。阅读器、进度条缩略图、AI 翻译用的都是排版后的页号。
- **导出时**：把处理结果固化成新的 CBZ，可以另存，也可以替换原文件（原文件进回收站）。

导入后在“设置 → 规则文件”里可以启用 / 停用、测试（选一本漫画看排版后的页面）、用记事本编辑（保存后下次打开漫画时生效）。

## 完整示例

```yaml
em: 1
kind: comic
id: manga-jp-layout
name: 日漫排版（右开、拆跨页、去白边）
version: 1.0.0

match:
  - series: '海贼|航海王'
  - path: '\\漫画\\日漫\\'

read:
  flow: h
  dir: rtl
  spread: double
  cover_alone: true
  fit: screen

pages:
  split_wide: { when_ratio: 1.3, order: rtl, keep_cover: true }
  trim: { color: auto, tolerance: 12, max: 8% }
  skip:
    - name: '(?i)(credit|recruit|招募|汉化组)'
    - last: 1
    - like: credits.png
  order: natural

export:
  format: jpg
  quality: 90
  max_height: 3000
  rename: '{序号:03}.{ext}'
  comicinfo: { manga: YesAndRightToLeft }
```

更多示例见 `tools/em-examples/manga-layout.em`、`tools/em-examples/webtoon-strip.em`。

## 头部

和其他 .em 规则一样：`em: 1`、`kind: comic`、`id`（小写字母、数字、`-`，2~40 个字符）、`name`，可选 `version`、`author`、`note`。漫画排版规则不能写 `hosts`。

正文只认 `match`、`read`、`pages`、`export` 四段；写错字段名会报错并给出行号。

## match：用于哪些漫画

列表，每项写一个或几个条件；**任一项满足**就套用这个规则（同一项里写了几个条件时要同时满足）。不写 `match` 或写成空列表表示所有漫画。

| 条件 | 说明 |
| --- | --- |
| `series: '正则'` | 作品名（书架上的系列名）匹配这个正则 |
| `path: '正则'` | 文件完整路径匹配这个正则。Windows 路径用 `\\` 表示一个反斜杠；路径分隔符写 `/` 或 `\\` 都能匹配 |
| `source: 规则 id` | 从某个下载源下载的漫画。**预留**：下载的文件还没有来源标记，目前这个条件不会命中任何漫画 |

正则是 Rust regex 语法，`(?i)` 开头表示不区分大小写。

有多个漫画排版规则都匹配同一本时，按规则 id 的字母顺序用第一个。

## read：阅读排版

打开漫画时的默认翻页方式。**只在这本书没有单独设置过时生效**（在阅读器里改过翻页方式、方向、单双页后，以你的设置为准）。都可以不写。

| 字段 | 取值 |
| --- | --- |
| `flow` | `h` 左右翻页 / `v` 上下翻页 / `vscroll` 上下连续滚动（条漫）/ `hscroll` 左右连续滚动 |
| `dir` | `ltr` 从左往右 / `rtl` 从右往左（日漫） |
| `spread` | `single` 单页 / `double` 双页并排 |
| `cover_alone` | `true` 双页时封面单独一页 |
| `fit` | `screen` 适应屏幕 / `width` 适应宽度 / `height` 适应高度 / `original` 原始大小 |
| `gap` | 连续滚动时的页间距，0~80（像素） |

## pages：页面处理

阅读和导出都会做。都可以不写；只写 `read` / `export` 的规则阅读时不改动页面。

### split_wide：拆跨页

扫描成一张图的跨页拆成两页。可以写 `split_wide: true` 用默认值，或：

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `when_ratio` | 1.3 | 宽 ÷ 高 超过这个数视为跨页（大于 1，不超过 5） |
| `order` | 跟 `read.dir`，没写为 `ltr` | `rtl` 先右半再左半；`ltr` 先左半再右半 |
| `keep_cover` | true | 第 1 页（跳过之后的第一页）不拆 |

从中线拆开；拆开后的两半各自再裁边。

### trim：裁白边 / 黑边

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `color` | auto | `white` 白边 / `black` 黑边 / `auto` 按页面四周的颜色自动判断 |
| `tolerance` | 12 | 色差容忍（0~255），扫描件发灰时调大 |
| `max` | 10% | **每边最多裁掉多少**（1%~45%），写 `8%`、`8` 或 `0.08` 都可以 |
| `sides` | 四边 | 只裁某几边，如 `[left, right]`；可选 `top` `bottom` `left` `right` |

从边缘往里，整行 / 整列都是边框色（允许 0.5% 的杂点）才裁；碰到画面就停，而且每边不超过 `max`，深色画面贴边时不会被裁坏。

### skip：跳过页面

列表，满足任一项的页面在阅读和导出时都去掉（原文件里不删除）。

| 写法 | 说明 |
| --- | --- |
| `- name: '正则'` | 页面文件名（不含压缩包里的文件夹）匹配 |
| `- first: N` | 每话（每个文件）开头 N 页，1~50 |
| `- last: N` | 每话最后 N 页，1~50 |
| `- like: credits.png` | 和样张图片相似的页（感知哈希 dHash）。可加 `distance: 10`（0~32，越小越严格，默认 10） |

`like` 的样张要放在**规则文件夹**里（和导入后的 .em 文件在一起，可以在“规则文件 → 编辑”打开的位置找到），只写文件名。样张不存在时导入确认框会提示，这一项暂时不起作用，其他设置照常生效。

### order：页面顺序

| 值 | 说明 |
| --- | --- |
| `natural`（默认） | 文件名自然排序（page2 在 page10 前面），和不用规则时一样 |
| `name` | 文件名字典序 |
| `keep` | 保持压缩包里的顺序（图片文件夹按文件名） |

## export：导出新 CBZ

在阅读器“排版”菜单里点“导出排版后的 CBZ…”，或在漫画书架多选后点“按规则导出”。先按 `pages` 处理，再做下面这些：

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `format` | keep | `keep` 保持原格式（处理过的页：JPEG 原图存 JPEG，其他存 PNG）/ `jpg` / `png`。**暂不支持导出 webp** |
| `quality` | 90 | JPEG 质量 1~100 |
| `max_height` | 不限 | 高度超过就等比缩小（200~30000 像素） |
| `rename` | `{序号:04}.{ext}` | 页面文件名模板：`{序号}`、`{序号:03}`（补零到 3 位）、`{ext}`、`{原名}`；也可以写 `{n}`、`{name}`。必须包含序号和扩展名 |
| `slice` | 不切 | 长图切成多页：`{ max_height: 2400, at_blank_rows: true }`。`at_blank_rows`（默认 true）只切在整行空白处；附近没有空白行时这一页会长一些，**不会切到画面** |
| `comicinfo` | — | `{ manga: YesAndRightToLeft }`（可选 YesAndRightToLeft / Yes / No），写进 ComicInfo.xml，其他阅读器也能认出右开。没写时 `read.dir: rtl` 的规则自动写 `YesAndRightToLeft` |

- 输出：`原文件夹/<原名> [排版].cbz`（重名时加 (2)、(3)…），自动加入媒体库。
- 替换原文件：只对 CBZ / ZIP。原文件移到回收站，新文件放在原来的位置（文件名不变），这本的阅读进度从头开始；图片文件夹只能另存。
- 导出的 CBZ 里有 `ComicInfo.xml`：标题、系列、页数、Manga，以及 `<Notes>em:<规则 id>@<版本></Notes>`。软件打开带这个标记的文件时，**不会再按同一个规则处理页面**（阅读排版照常），避免拆两次、裁两次；也不能再按同一个规则导出。

## 限制

- PDF 不处理。avif 或损坏的图片原样显示 / 原样导出。
- 第一次打开要拆跨页、按样张跳过的漫画时，要先读一遍所有页面（多线程，几百页通常几秒），结果会缓存到关闭软件为止。
- AI 翻译按排版后的页号记录。改了规则后页号会变，之前的译文可能对不上页面。
- 阅读时处理过的页面重新编码为 JPEG（质量 92），原图是 PNG 时用 PNG。
