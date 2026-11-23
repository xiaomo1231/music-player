# 示例 .em 规则：条漫（长图）排版
em: 1
kind: comic
id: webtoon-strip
name: 条漫：无缝滚动、切长图
version: 1.0.0
match:
  - path: '\\漫画\\条漫\\'
read:
  flow: vscroll
  fit: width
  gap: 0
pages:
  trim: { color: white, tolerance: 8, max: 5%, sides: [left, right] }
export:
  slice:                # 超长图切成多页，切在整行空白处，不切到画面
    max_height: 2400
    at_blank_rows: true
  format: jpg
  quality: 88
