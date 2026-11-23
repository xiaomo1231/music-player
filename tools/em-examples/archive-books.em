# 示例 .em 规则：Internet Archive 上公有领域 / CC 授权的电子书
# 只作写法示例。规则只能访问 hosts 里列出的域名，不能运行脚本。
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
      https://archive.org/advancedsearch.php?q=({{query | url_encode}})%20AND%20mediatype:texts%20AND%20(licenseurl:*publicdomain*%20OR%20licenseurl:*creativecommons.org*)&fl[]=identifier&fl[]=title&fl[]=creator&fl[]=year&rows=30&page={{page}}&output=json
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
  # 没有 EPUB 时给用户看的话
  on_empty: 这个条目没有 EPUB 文件
