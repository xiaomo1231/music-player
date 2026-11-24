# .em 规则模板：音乐下载源（网址和选择器是占位的，换成你有权使用的网站，比如自建曲库）
em: 1
kind: source
id: my-em-source
name: 我的规则下载源
version: 0.1.0
author: 你的名字
note: 模板：网址和选择器是占位的
media_kinds: [music]
hosts: [example.org]
login: none
test:
  query: 测试
search:
  request:
    url: 'https://example.org/search?q={{query | url_encode}}&page={{page}}'
  list: 'css: .song'
  fields:
    id:     'css: a.title @href'
    title:  'css: a.title'
    artist: 'css: .artist'
    album:  'css: .album'
resolve:
  request:
    url: '{{item.id}}'
  target:
    type: http
    url: 'css: a.download @href | abs_url'
    ext: mp3
  on_empty: 这首歌没有下载地址
