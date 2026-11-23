# 示例 .em 规则：按字幕组发布名整理番剧（不联网，只在“整理文件”扫描时用）
# 从上往下试，第一种匹配上就用它；都不匹配的文件交给软件自带的识别
em: 1
kind: organize
id: anime-release
name: 字幕组发布名
version: 1.0.0

rules:
  # [字幕组] 作品名 第二季 - 05 [1080p].mkv、[字幕组] 作品名 S2 - 05.mkv
  - match: '^\[(?P<group>[^\]]+)\]\s*(?P<series>.+?)\s+(?:第|S)(?P<season>[0-9一二三四五六七八九十]+)季?\s*-\s*(?P<episode>\d+(?:\.5)?)'
    media: anime

  # [字幕组] 作品名 - 05 [1080p].mkv
  - match: '^\[(?P<group>[^\]]+)\]\s*(?P<series>.+?)\s+-\s+(?P<episode>\d+(?:\.5)?)(?:v\d)?\b'
    media: anime
    template: '番剧/{作品名}/Season {季:02}/{作品名} - S{季:02}E{集:02}.ext'
