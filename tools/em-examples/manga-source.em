# 示例 .em 规则：漫画下载源（网址是占位的，换成你有权使用的网站，比如自建漫画库）
em: 1
kind: source
id: my-comics
name: 我的漫画库
version: 1.0.0
media_kinds: [manga]
hosts: [comics.example.org, img.example.org]
login: optional
test:
  query: 星屑邮局
search:
  request:
    url: 'https://comics.example.org/search?q={{query | url_encode}}'
  list: 'css: .book-item'
  fields:
    id:     'css: a @href'
    title:  'css: .title'
    series: 'css: .title'
    artist: 'css: .author'
    cover:  'css: img @src'
episodes:
  request:
    url: '{{item.id}}'
  list: 'css: #chapters li a'
  fields:
    id:      'css: @href'
    title:   'css: @text'
    episode: 'css: @text | regex(第(\d+)话, 1) | float'
resolve:
  request:
    url: '{{item.id}}'
  target:
    type: pages
    urls: 'css: .reader img @data-src | abs_url'
    headers: { Referer: 'https://comics.example.org/' }
  on_empty: 这一话没有图片
