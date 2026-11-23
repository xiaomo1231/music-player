# 示例 .em 规则：小说重新排版（不联网，只处理本地已入库的小说）
em: 1
kind: clean
id: novel-layout
name: 通用小说排版
version: 1.0.0
scope: all            # 也可以写 { title: "潮汐档案" } 只作用于某本书

remove:
  - line: '^(本章完|未完待续)[。！!]*$'
  - regex: '本章未完请翻页'       # 去广告规则先去掉空白和标点再匹配，所以不写括号和逗号

replace:
  - { from: '\.{3,}', to: '……' }
  - { from: '\s+([，。！？；：])', to: '$1' }

layout:
  merge_broken_lines: true     # 把 TXT 里一句话中间的硬换行接起来
  indent: "　　"               # 段首两个全角空格
  blank_lines: 0
  punctuation: fullwidth
  split_chapters: '^\s*第[〇零一二三四五六七八九十百千万0-9]+[章节回卷].*$'
