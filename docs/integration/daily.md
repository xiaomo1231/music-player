# 每日推荐（daily）集成说明

## 1. 需要在 `lib.rs` 的 `generate_handler!` 里注册的命令

```
daily::daily_get
daily::daily_regenerate
daily::daily_feedback
daily::daily_history
daily::daily_save_playlist
daily::daily_ai_enhance
```

`init_modules` 和 `setup_modules` 里已经有 `daily::init` / `daily::setup`，不用再改。

## 2. 依赖

- 不需要新的 Cargo 或 npm 依赖。
- 本地时间用 Windows 的 `GetLocalTime`（kernel32，和 `scanner.rs` 的写法一样），不引入日期库。
- 随机数用内置的 xorshift，不引入 `rand`。

## 3. 需要的前端改动

- 无。`App.svelte` 已经挂载 `Daily`，`ui.svelte.ts` 已经有 `daily` 视图。
- 前端文件：`src/lib/daily.ts`、`src/views/Daily.svelte`、`src/components/daily/DailyBanner.svelte`、`src/components/daily/DailyStats.svelte`。

## 4. 命令说明

| 命令 | 参数 | 返回 |
| --- | --- | --- |
| `daily_get` | `day?: string`（YYYY-MM-DD，缺省为今天） | `Doc`：`{day, title, summary, tracks:[{id, reason, kind}], stats, ai, generated_at}` |
| `daily_regenerate` | 无 | 今天的新一批（避开昨天和本批已有的歌） |
| `daily_feedback` | `file_id`、`kind: 'like' \| 'not_song' \| 'not_artist' \| 'not_today'` | 无 |
| `daily_history` | 无 | 最近 7 天的 `{day, title, summary, count}` 列表，新到旧 |
| `daily_save_playlist` | `day`、`name`（空串用“每日推荐 M-D”） | 新歌单 id |
| `daily_ai_enhance` | `day` | 更新后的 `Doc`（`ai: true`）；没有配置 AI 时返回“还没有配置 AI 服务”的错误 |

`kind` 取值：`sim` 相似、`old` 旧爱、`never` 没听过、`explore` 探索。

## 5. 数据

- `daily` 表：`day` 主键，`data` 为 `Doc` 的 JSON。每天第一次 `daily_get` 时生成；写入后删除早于 7 天（含今天）的行。
- 库里不足 10 首时不生成、不保存，返回 `tracks` 为空、`summary` 为提示语的 `Doc`。
- 反馈写入 `feedback` 表（已有）。`not_song` 永久排除；`not_today` 排除 3 天；`not_artist` 排除整个艺人；`like` 的艺人在相似推荐里加分。

## 6. 推荐规则（纯本地）

- 候选分组（按顺序归类，每首只进一组）：
  1. 相似：艺人在最近 7 天常听的前 5 名里（喜欢过的艺人加分），或风格与最近常听的前 3 个风格相同。
  2. 旧爱：播放次数 ≥ 3，且 90 天没听（或从没记录上次播放）。
  3. 没听过：播放次数为 0，且没有上次播放时间。
  4. 探索：其余的。
- 名额按 40% / 30% / 20% / 10% 分配；某一组不够时，缺的名额按“没听过 → 探索”的顺序补给还有候选的组。
- 常被跳过的歌（累计跳过 ≥ 3 次且超过播放次数的两倍，或近 90 天一半以上被跳过）权重降到 0.3。
- 排列时同一艺人尽量不相邻。
- 昨天推过的歌今天不再出现；如果排除后一首都不剩，会放宽这一条。

## 7. 统计（`Doc.stats`）

- `yesterday` / `yesterday_plays` / `yesterday_seconds` / `skipped_yesterday`：昨天的播放（不含跳过）。
- `top_tracks_7d`、`top_artists_7d`（各 5 个）、`top_tracks_30d`（15 个）、`top_artists_30d`（8 个）。
- `plays_7d`、`skipped_7d`、`listen_seconds_7d`：最近 7 天的收听。“总时长”取的是最近 7 天的收听时长。
- `hours`：近 30 天每小时的播放次数（本地时间，长度 24）；`peak_label` 为最常听的时段，例如 “深夜 22 点左右”。

## 8. 已知问题和未做的部分

- **模式二（加入新歌并下载）未实现**：界面里是禁用的选项，依赖“自动获取”模块。
- **声音特征（bliss-audio）未使用**：没有引入依赖。“相似”只按艺人和风格判断。
- **定时生成未实现**：只在第一次请求时生成，不在后台定时跑。
- 时段统计用的是调用时的本地时区偏移，没有逐条考虑历史记录所在季节的夏令时差异（中国大陆没有夏令时，影响可忽略）。
- 对外的 `Stats` 只给每类前几名；AI 点评只发送 30 天内前 15 首、前 8 位艺人和今天推荐的歌名、艺人。

## 9. 自测

- `cargo test --lib`：daily 模块 9 个测试通过（日期换算、比例分配、同艺人不相邻、排列、昨天排除与 7 天清理、反馈排除、库太小不保存、统计、AI 结果写回）。整个 crate 在临时副本里 104 个测试全部通过、无警告（副本只替换了 `fetch.rs`，见 organize.md）。
- `pnpm check`：本模块文件零错误零警告。
- `pnpm build`：通过。
