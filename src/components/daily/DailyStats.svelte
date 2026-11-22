<script lang="ts">
  import { fmtDur, type DailyStats } from '../../lib/daily';

  let { stats }: { stats: DailyStats } = $props();

  const cards: [string, string][] = $derived([
    [
      `${stats.yesterday_plays} 首`,
      stats.yesterday_plays ? `昨日播放 · ${fmtDur(stats.yesterday_seconds)}` : '昨天没有播放记录',
    ],
    [stats.top_artists_7d[0]?.name ?? '—', '最近 7 天最常听'],
    [stats.peak_label ?? '—', '最常听歌的时段'],
    [fmtDur(stats.listen_seconds_7d), `最近 7 天收听 · ${stats.plays_7d} 次`],
    [`${stats.skipped_yesterday} 首`, '昨天被跳过，已降权'],
  ]);
</script>

<div class="dstats">
  {#each cards as c, i (i)}
    <div><b>{c[0]}</b>{c[1]}</div>
  {/each}
</div>
