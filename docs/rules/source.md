# 下载源规则（.em，kind: source）

不写代码，用一个文本文件告诉软件“怎么在某个网站搜索、怎么拿到下载地址”。导入后它就是一个普通的下载源：出现在“自动获取 → 我的来源 → 可添加的下载源”里，搜索结果进搜索页，下载走同一个队列，下完补标签、整理入库。

> 只给你有权获取内容的网站写规则：官方开放接口、自己的 NAS / 网盘、公有领域或 CC 授权的书库等。规则引擎不会登录、不会过验证码、不能运行脚本、不能做加密签名，也不会绕过付费或会员限制。遇到 401 / 403、登录页或验证码页时直接停下，提示“需要登录或验证码”。

实现：`src-tauri/src/em/source.rs`（阶段和下载方式）、`em/expr.rs`（取值表达式）、`em/net.rs`（联网限制）。

## 1. 一个完整的例子

`tools/em-examples/archive-books.em`：在 Internet Archive 搜索公有领域 / CC 授权的电子书，下载 EPUB（下面的搜索网址省略了授权条件，完整写法见文件）。

```yaml
em: 1
kind: source
id: archive-books
name: Internet Archive 公版书（示例）
version: 1.0.0
author: 示例
note: 只搜索标了公有领域或 CC 授权的电子书，下载 EPUB
media_kinds: [novel]
hosts: [archive.org]
login: none

test:
  query: pride and prejudice
  min_results: 3

search:
  request:
    url: >-
      https://archive.org/advancedsearch.php?q=({{query | url_encode}})%20AND%20mediatype:texts&fl[]=identifier&fl[]=title&fl[]=creator&fl[]=year&rows=30&page={{page}}&output=json
  list: "json: $.response.docs[*]"
  fields:
    id:      "json: $.identifier"
    title:   "json: $.title"
    series:  "json: $.title"
    artist:  "json: $.creator | first | default(佚名)"
    year:    "json: $.year | int"
    cover:   "tpl: https://archive.org/services/img/{{id}}"
    media_kind: "const: novel"

resolve:
  request:
    url: "https://archive.org/metadata/{{item.id | url_encode}}"
  vars:
    file: "json: $.files[*].name | match(\\.epub$) | first"
  target:
    type: http
    url: "tpl: https://archive.org/download/{{item.id | url_encode}}/{{file | url_encode}}"
    ext: epub
  on_empty: 这个条目没有 EPUB 文件
```

流程：`search` 请求搜索接口 → 按 `list` 拆成一条条结果 → 每条按 `fields` 取出标题、作者等 → 用户点下载时 `resolve` 请求这一条的详情 → 算出 `vars` → 按 `target` 给出下载地址。

## 2. 头部

| 字段 | 说明 |
| --- | --- |
| `em` | 格式版本，写 `1` |
| `kind` | `source` |
| `id` | 小写字母、数字、`-`。注册成下载源 `em-<id>` |
| `name` / `version` / `author` / `note` | 显示用；`note` 会出现在“可添加的下载源”的提示里 |
| `media_kinds` | 支持的类型：`music` / `video` / `anime` / `manga` / `novel`；不写表示全部 |
| `hosts` | **必填**。允许访问的网站，见第 6 节 |
| `login` | `none`（默认）/ `optional` / `required`，见第 6 节 |
| `test` | 测试面板用：`query`（默认关键字）、`min_results`（少于这么多条时提示） |

## 3. 阶段和步骤

| 阶段 | 必填 | 作用 |
| --- | --- | --- |
| `search` | 是 | 按关键字搜索，得到作品 / 文件列表 |
| `episodes` | 否 | 一部作品 → 各集 / 各话 / 各章。写了以后，搜索页按“来源 + 作品”显示作品卡，点开作品卡时才取这部作品的各集；追更检查时把前 `max_works` 部作品（默认 3，最多 10）展开成各集（从作品卡追更的直接展开那一部） |
| `resolve` | 是 | 一条结果 → 下载方式（不翻页、不列表，算 `vars` 和 `target`） |

每个阶段由这些字段组成：

```yaml
request:                 # 请求哪个网址（resolve 里可以不写：直接用搜索结果里的值）
  url: 'https://x.org/s?q={{query | url_encode}}&p={{page}}'
  method: GET            # GET（默认）或 POST
  form: { kw: '{{query}}' }        # POST 表单，和 json 二选一
  json: { keyword: '{{query}}' }   # POST JSON，字符串里可以用模板
  headers: { Referer: 'https://x.org/' }   # 只能写 Referer、Accept、Accept-Language
  encoding: auto         # auto（默认：看响应头、网页 meta、再自动识别）/ utf-8 / gbk / big5 …
list: 'css: .result li'  # 每条结果在哪里（search / episodes 必填）
fields:                  # 每条结果取哪些字段（search / episodes）
  title: 'css: a'
next: 'css: a.next @href'   # 下一页地址；跟着翻页
max_pages: 5             # 最多翻几页，默认 5，最多 50
vars:                    # 中间值（episodes / resolve），后面用 {{名字}} 引用
  file: 'json: $.files[*].name | first'
target: { … }            # 下载方式（只在 resolve 里）
on_empty: 取不到时给用户看的话
```

- `request.url` 里的相对地址按上一页补全；跳到看过的页面、某页没有结果、到了 `max_pages` 都会停止翻页。
- `search` 的 `url` 用了 `{{page}}` 时，搜索页要第 2 页会照着请求；没用时只有第 1 页（靠 `next` 翻）。

### 字段

| 字段 | 说明 |
| --- | --- |
| `id` | 在这个网站里的唯一编号，**要稳定**（同一条每次搜都一样）。不写时用标题等算一个 |
| `title` | 必填，取不到标题的条目会跳过 |
| `series` | 作品名（番剧 / 漫画 / 小说用，搜索页按它分组）。`episodes` 里不写时用作品的标题 |
| `artist` / `album` | 作者 / 艺人、专辑 |
| `year` / `track` / `season` | 整数（会自动取文字里的第一个数字） |
| `episode` | 第几集 / 话 / 章，可以是小数 |
| `duration` | 时长，`205`、`3:25`、`1:02:03` 都行 |
| `cover` | 封面网址，相对地址会补全 |
| `media_kind` | 不写时用搜索页当前选的类型 |
| `size` / `quality` | 大小（字节，可配合 `size` 过滤器）和清晰度文字，显示在搜索结果里 |
| `extra.名字` | 自己用的值，原样带到后面的阶段：`{{item.extra.名字}}` |

软件会自动在 `extra._page` 里记下这条结果所在页面的网址，`resolve` 里的相对地址按它补全。`episodes` 的每一集会带上作品的 `extra`（同名的以集为准）。

## 4. 取值表达式

写法：`<取值方式>: <参数> | 过滤器 | 过滤器 …`。分隔过滤器的 `|` **前面要有空格**（正则里的 `a|b` 不受影响）。

| 取值方式 | 说明 |
| --- | --- |
| `css: 选择器 @属性` | 网页里按 CSS 选择器取，每个匹配到的元素一个值。`@text`（默认，块级元素和 `<br>` 会换行）、`@html`、`@href`、`@src` 等任意属性。只写 `@href` 表示当前元素本身 |
| `json: JSONPath` | JSON 里按 JSONPath（RFC 9535）取，如 `$.data.items[*].name`；可以省略开头的 `$.`。取到数组会展开成多个值 |
| `regex: 正则` | 在原文（当前元素的 HTML、或整页内容）里找，取第 1 组括号，没有括号取整个匹配，所有匹配都要 |
| `tpl: 文字` | 模板，`{{变量}}` 换成变量值 |
| `const: 值` | 固定值 |

没写取值方式的字符串当作模板（例如 `ext: epub`）。

在 `list` 里取到的每一项，是 `fields` 取值时的“当前位置”：`list: 'css: li'` 之后 `title: 'css: a'` 只在这个 `li` 里找；`list: 'json: $.items[*]'` 之后 `title: 'json: $.name'` 从这一项开始找。`list` 和 `next` 只能用 `css:` / `json:` / `regex:`。

字段一般只要第一个值；`target.urls`（漫画图片）和小说正文要全部的值。

### 模板变量

| 变量 | 在哪里能用 |
| --- | --- |
| `{{query}}`、`{{page}}` | search |
| 同一条里的其他字段：`{{id}}`、`{{title}}` … | fields（引用顺序不限） |
| `{{item.id}}`、`{{item.title}}`、`{{item.extra.名字}}` … | episodes（作品）、resolve（要下载的那一条） |
| `vars` 里的名字 | 同一阶段后面的 vars 和 target |
| `{{chapter.url}}`、`{{chapter.title}}`、`{{chapter.index}}` | 小说正文的 content |

模板里也能用过滤器：`{{query | url_encode}}`。变量没有值时报“模板里的 {{x}} 没有值”，可以用 `{{x | default(…)}}` 兜底；在 `resolve` 的下载地址里遇到没有值的变量，按“取不到”处理并显示 `on_empty`。

### 过滤器

| 过滤器 | 作用 |
| --- | --- |
| `trim` | 去掉首尾空白 |
| `replace(原文, 换成)` | 文字替换（不是正则）。含逗号、空格的参数用引号括起来：`replace(', ', /)` |
| `regex(正则, 组号)` | 取正则的某一组（默认第 1 组），不匹配的值去掉 |
| `match(正则)` | 只留匹配正则的值 |
| `split(分隔符)` | 拆成多个值（去掉空的） |
| `join(分隔符)` | 多个值合成一个 |
| `first` / `last` | 第一个 / 最后一个值 |
| `int` / `float` | 取文字里的第一个数字（`第 1,234 话` → `1234`） |
| `abs_url` | 相对地址补全成完整网址 |
| `url_encode` | 百分号编码（空格是 `%20`） |
| `strip_html` | 去掉 HTML 标签，保留文字和换行 |
| `size` | `1.2 GB` → 字节数 |
| `date` | `2024年3月5日`、`2024/3/5`、时间戳 → `2024-03-05` |
| `default(值)` | 没有值（或都是空的）时用这个值 |

没有加密、签名、哈希、Base64、eval 之类的过滤器，以后也不会加。

## 5. 下载方式（resolve.target）

| type | 写法 | 下载 |
| --- | --- | --- |
| `http` | `url`，可选 `ext`（不写时看网址的扩展名） | 直链下载，断点续传 |
| `ytdlp` | `url`：视频页地址 | 交给 yt-dlp |
| `magnet` | `url`：`magnet:?…` 或 .torrent 网址 | BT 下载 |
| `pages` | `urls`：全部图片地址（一个表达式取多个值），可选 `headers: { Referer: … }` | 按顺序下载后打包成 CBZ。resolve 写了 `next` 时跟着翻页收集图片 |
| `chapters` | `toc` + `content`，见下 | 按章取正文，生成 EPUB |

所有下载地址也必须在 `hosts` 里（磁力链接除外），图片放在别的域名（CDN）时把它也写进 `hosts`。

小说：

```yaml
resolve:
  request:
    url: 'https://n.example.org/book/{{item.id}}/'
  target:
    type: chapters
    toc:                       # 目录：不写 request 时就用 resolve 请求到的这一页
      list: 'css: #toc a'
      title: 'css: @text'
      url: 'css: @href'
      next: 'css: a.next-toc @href'    # 目录分页（可选）
    content:                   # 每一章：不写 request 时直接 GET 章节地址
      text: 'css: #content'
      next: 'css: a.next-part @href'   # 一章分几页时（可选），max_pages 默认 5
  on_empty: 没有正文
```

重复的章节地址会去掉。整本书一次生成，章节多时会比较慢（每个网站每秒最多一个请求）。

## 6. 联网限制

- 只能访问 `hosts` 里的网站：`example.org` 只匹配它自己；`*.example.org` 只匹配子域名（主域名要另外写）；带端口的 `nas.local:8080` 只匹配这个端口，不带端口的只匹配默认端口。
- 每一次跳转、最终地址、下载地址都要在 `hosts` 里；只允许 http / https，网址里不能带用户名密码。
- 本机、内网地址（`localhost`、`127.0.0.1`、`192.168.x.x`、`10.x.x.x`、`169.254.x.x`、IPv6 本地地址等）只有在 `hosts` 里原样写出时才能访问；通配不算。域名解析到内网地址时同样拦下。
- 只能 GET / POST；请求头只能写 `Referer`、`Accept`、`Accept-Language`。
- 单个响应最多 8 MB，30 秒超时；同一网站两次请求至少间隔 1 秒；一次搜索或一次解析最多 2000 个请求。
- `login: none` 不带任何 Cookie；`optional` / `required` 时带上你自己在“网站登录”里登录后保存的 Cookie（规则本身不能写 Cookie 或 Authorization）。`required` 而你还没登录时提示先去登录。
- 遇到 401 / 403，或者结果为空而页面是登录页、验证码页时，停下并提示，不会尝试绕过。

## 7. 测试和排错

在“设置 → 规则文件”里点规则的“测试”：输入关键字（留空用 `test.query`）点“运行”，会依次显示：

1. 搜索：每个请求的网址和 HTTP 状态、每页取到几条、前 3 条的字段；
2. 分集（写了 `episodes` 时）：第一部作品的各集；
3. 解析下载地址：第一条结果的下载方式（直链地址、图片张数、章节数和第一章正文前 200 字），**不会真的下载**。

出错的步骤标红并显示原因。导入时规则写错的地方会带行号提示（如“第 12 行：search 里不认识的字段 titel”）。改规则：点“编辑”用记事本打开，保存后下次使用时自动生效。

## 8. 什么时候不要用规则

需要执行 JavaScript、计算签名 / 加密参数、走复杂登录流程、或者接口经常变化的网站，规则写不出来，也不应该硬写。这类情况请用**外部程序下载源**（自己写的 exe，见 [下载源接口.md](../下载源接口.md) 第 9 节）或在软件里正式接入（实现 `Provider`）——前提仍然是你有权获取这些内容。
