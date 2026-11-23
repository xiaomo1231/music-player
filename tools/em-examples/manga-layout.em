# 示例 .em 规则：漫画排版（不联网，只处理本地漫画）
em: 1
kind: comic
id: manga-jp-layout
name: 日漫排版（右开、拆跨页、去白边）
version: 1.0.0

# 作用范围：满足任一条件的漫画套用这份规则；都不写表示所有漫画
match:
  - series: '.*'                 # 作品名正则
  - path: '\\漫画\\日漫\\'        # 文件路径正则
  - source: my-comics            # 从某个下载源下载的（写下载源规则的 id）

# 一、阅读排版：打开时生效，不改文件
read:
  flow: h               # h 左右翻页 / v 上下翻页 / vscroll 条漫滚动 / hscroll
  dir: rtl              # 从右往左
  spread: double        # 双页并排
  cover_alone: true     # 封面单独一页，后面两两成对
  fit: screen
  gap: 0                # 条漫页间距（px）

# 二、页面处理：阅读时实时做，也可以“导出新 CBZ”时固化
pages:
  split_wide:           # 拆开扫描成一张图的跨页
    when_ratio: 1.3     # 宽 / 高 超过 1.3 视为跨页
    order: rtl          # 拆开后先右半再左半
    keep_cover: true    # 第 1 页不拆
  trim:                 # 裁掉白边 / 黑边
    color: auto         # auto / white / black
    tolerance: 12       # 0-255 色差容忍
    max: 8%             # 每边最多裁掉 8%，防止把画面裁坏
  skip:                 # 跳过（不删除）这些页
    - name: '(?i)(credit|recruit|招募|汉化组)'   # 页面文件名正则
    - last: 1                                     # 每话最后 1 页
    - like: credits.png                           # 和样张相似的页（感知哈希）
  order: natural        # natural 自然排序 / name 字典序 / keep 保持压缩包内顺序

# 三、导出新 CBZ 时额外做的（原文件保留，可撤销）
export:
  format: jpg           # keep 保持原格式 / jpg / png
  quality: 90
  max_height: 3000      # 超过就等比缩小
  rename: '{序号:03}.{ext}'
  auto: false           # true：从下载源下载完成后自动导出，替换刚下载的文件
  comicinfo:            # 写入 ComicInfo.xml，其他阅读器也能认出右开
    manga: YesAndRightToLeft
