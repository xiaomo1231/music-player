<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../../lib/Icon.svelte';
  import { invoke, inTauri, type Series, type Video } from '../../lib/api';
  import { ui, toast } from '../../lib/ui.svelte';
  import { lib } from '../../lib/library.svelte';
  import { fmtT, hueOf, coverCSS } from '../../lib/util';
  import { posterBg, subBadge, fmtEp, thumbUrl, lazyThumb } from '../../lib/media.svelte';

  // 番剧 / 电视剧共用的剧集详情页：横幅、继续观看、季切换、每集卡片
  let {
    series,
    kind,
    onback,
    onchanged,
  }: { series: Series; kind: 'anime' | 'tv'; onback: () => void; onchanged?: () => void } = $props();

  let root = $state<HTMLElement>();
  let eps = $state<Video[]>([]);
  let loaded = $state(false);
  let season = $state<number | null>(null);
  let thumbs = $state<Record<number, string>>({});

  const sOf = (e: Video) => e.season ?? 1;
  const seasons = $derived(
    [...new Set(eps.map(sOf))].sort((a, b) => (a === 0 ? 999 : a) - (b === 0 ? 999 : b)),
  );
  // 默认选中“继续观看”所在的季，否则第一季
  const curSeason = $derived(
    season != null && seasons.includes(season)
      ? season
      : (eps.find(e => e.id === series.next_id) ? sOf(eps.find(e => e.id === series.next_id)!) : (seasons[0] ?? 1)),
  );
  const curEps = $derived(eps.filter(e => sOf(e) === curSeason));
  const missing = $derived(series.missing_subs > 0);
  const started = $derived(series.watched > 0 || series.last_played != null);

  async function load() {
    if (!inTauri) {
      loaded = true;
      return;
    }
    eps = await invoke<Video[]>('series_episodes', { id: series.id });
    loaded = true;
  }
  // 媒体库变化、关闭播放器（进度可能变了）时重新读取
  $effect(() => {
    void lib.version;
    void ui.videoId;
    void series.id;
    load();
  });

  onMount(() => {
    // 从列表点进来时滚回顶部
    root?.closest('.lib')?.scrollTo(0, 0);
  });

  const epLabel = (e: Video) => (sOf(e) === 0 ? `特别篇 ${fmtEp(e.episode)}` : `第 ${fmtEp(e.episode)} 集`);
  const progress = (e: Video) => (e.watched ? 100 : e.duration > 0 ? Math.min(100, (e.position / e.duration) * 100) : 0);
  const epBg = (e: Video) => {
    const t = thumbUrl(thumbs[e.id] ?? e.thumb);
    return t ? `url("${t}")` : coverCSS(hueOf(series.title + e.id), (e.episode ?? 0) % 2 === 1);
  };

  function idsOf(s: number): number[] {
    return eps.filter(e => sOf(e) === s).map(e => e.id);
  }

  function play(e: Video) {
    ui.openItem(e.id, kind === 'anime' ? 'anime' : 'video', idsOf(sOf(e)));
  }

  function resume() {
    if (series.next_id == null) {
      // 全部看完：从第一集重新看
      if (eps[0]) play(eps[0]);
      return;
    }
    const ne = eps.find(e => e.id === series.next_id);
    if (ne) play(ne);
    else ui.openItem(series.next_id, kind === 'anime' ? 'anime' : 'video', [series.next_id]);
  }

  const resumeText = $derived(
    series.next_id == null ? '重新观看' : started ? `继续观看 ${series.next_label ?? ''}` : `开始观看 ${series.next_label ?? ''}`,
  );

  async function markSeason() {
    const ids = idsOf(curSeason);
    if (!ids.length || !inTauri) return;
    await invoke('set_watched', { ids, watched: true });
    toast(curSeason === 0 ? '已标记特别篇为看完' : `已标记第 ${curSeason} 季为看完`);
    await load();
    onchanged?.();
  }

  function genSub(e: Video) {
    ui.subVideo = e.id;
    ui.go('aisub');
  }
</script>

<div class="series-detail" bind:this={root}>
  <button class="mp-chip" onclick={onback}>← 返回{kind === 'anime' ? '番剧' : '电视剧'}列表</button>

  <div class="banner" style:margin-top="12px" style:background-image={posterBg(series.poster, series.thumb, series.title)}>
    <div class="pst" style:background-image={posterBg(series.poster, series.thumb, series.title, true)}></div>
    <div>
      <h2>{series.title}</h2>
      <div class="acts" style="margin-bottom:10px">
        <span class="badge2">{kind === 'anime' ? '番剧' : '电视剧'}</span>
        <span class="badge2">{series.seasons > 1 ? `${series.seasons} 季 · ` : ''}共 {series.episodes} 集</span>
        <span class="badge2">已看 {series.watched} 集</span>
        <span class="badge2 {missing ? 'no' : 'ok'}">{missing ? `${series.missing_subs} 集缺字幕` : '字幕齐全'}</span>
      </div>
      <div class="acts">
        <button class="mp-btn" onclick={resume} disabled={!eps.length}><Icon name="play" /> {resumeText}</button>
        <button class="mp-btn ghost" onclick={markSeason} disabled={!curEps.length}>标记本季已看完</button>
      </div>
    </div>
  </div>

  {#if seasons.length}
    <div class="tabs">
      {#each seasons as s (s)}
        <button class="mp-chip" class:on={s === curSeason} onclick={() => (season = s)}>{s === 0 ? '特别篇' : `第 ${s} 季`}</button>
      {/each}
      <span class="badge2">本季 {curEps.length} 集 · 已看 {curEps.filter(e => e.watched).length} 集</span>
    </div>
  {/if}

  {#if !loaded}
    <div class="empty">正在读取剧集…</div>
  {:else if !curEps.length}
    <div class="empty">没有找到剧集。</div>
  {:else}
    <div class="eps">
      {#each curEps as e (e.id)}
        {@const b = subBadge(e.subs)}
        <div
          class="ep"
          role="button"
          tabindex="0"
          onclick={() => play(e)}
          onkeydown={ev => ev.key === 'Enter' && play(e)}
          use:lazyThumb={{ id: e.id, thumb: e.thumb, onthumb: n => (thumbs[e.id] = n) }}
        >
          <div class="th" style:background-image={epBg(e)}>
            <b>{fmtEp(e.episode)}</b>
            <em><Icon name="play" /></em>
            <u>{fmtT(e.duration)}</u>
            <i style:width="{progress(e)}%"></i>
          </div>
          <p>{epLabel(e)}{e.title && e.title !== series.title ? `　${e.title}` : ''}</p>
          <div class="row">
            <span class="badge2 {b.cls}">{b.text}</span>
            {#if e.watched}<span class="badge2">已看</span>{/if}
            {#if e.subs === 'none'}
              <button
                onclick={ev => {
                  ev.stopPropagation();
                  genSub(e);
                }}>生成字幕</button
              >
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>
