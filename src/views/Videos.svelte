<script lang="ts">
  import MoreButton from '../components/MoreButton.svelte';
  import { openMenu, mediaEntries } from '../lib/items.svelte';
  import Poster from '../components/media/Poster.svelte';
  import SeriesDetail from '../components/media/SeriesDetail.svelte';
  import { lib } from '../lib/library.svelte';
  import { ui } from '../lib/ui.svelte';
  import { invoke, inTauri, type Series, type Video } from '../lib/api';
  import { fmtT } from '../lib/util';
  import { lazyThumb, seriesBadge } from '../lib/media.svelte';

  let videos = $state<Video[]>([]);
  let shows = $state<Series[]>([]);
  let loaded = $state(false);
  let openId = $state<number | null>(null);
  let thumbs = $state<Record<number, string>>({});

  const openSeries = $derived(openId == null ? null : (shows.find(s => s.id === openId) ?? null));
  const pct = (s: Series) => (s.episodes ? (s.watched / s.episodes) * 100 : 0);
  const vBadge = (v: Video) => (v.watched ? '已看完' : v.position > 5 ? `看到 ${fmtT(v.position)}` : '未观看');
  const vPct = (v: Video) => (v.watched ? 100 : v.duration > 0 ? Math.min(100, (v.position / v.duration) * 100) : 0);

  async function load() {
    if (!inTauri) {
      loaded = true;
      return;
    }
    const [v, s] = await Promise.all([invoke<Video[]>('list_videos'), invoke<Series[]>('list_series', { kind: 'tv' })]);
    videos = v;
    shows = s;
    loaded = true;
  }
  $effect(() => {
    void lib.version;
    void ui.videoId;
    load();
  });

  // 搜索结果跳过来时直接打开那部电视剧
  $effect(() => {
    const sid = ui.seriesId;
    if (sid == null || ui.view !== 'video' || !loaded) return;
    if (shows.some(s => s.id === sid)) {
      openId = sid;
      ui.seriesId = null;
    }
  });

  // 独立视频：播放时列表就是当前海报墙的顺序
  function playVideo(v: Video) {
    ui.openItem(v.id, 'video', videos.map(x => x.id));
  }
  function seriesMenu(e: MouseEvent, s: Series) {
    openMenu(e, s.title, mediaEntries('series', s.id, s.title, [
      { label: '打开详情', run: () => (openId = s.id) },
      ...(s.next_id != null ? [{ label: '继续观看', run: () => s.next_id != null && ui.openItem(s.next_id, 'video') }] : []),
    ]));
  }
  function videoMenu(e: MouseEvent, v: Video) {
    openMenu(e, v.title, mediaEntries('video', v.id, v.title, [
      { label: '播放', run: () => playVideo(v) },
      { label: '生成 AI 字幕', run: () => { ui.subVideo = v.id; ui.go('aisub'); } },
      { label: '转换格式…', run: () => { ui.convertPaths = [v.path]; ui.go('convert'); } },
      { label: '整理文件…', run: () => ui.go('org') },
    ]));
  }
</script>

<section class="lib on">
  {#if openSeries}
    <SeriesDetail series={openSeries} kind="tv" onback={() => (openId = null)} onchanged={load} />
  {:else}
    <header>
      <div>
        <h2>视频</h2>
        <p>{shows.length} 部电视剧 · {videos.length} 个独立视频</p>
      </div>
    </header>

    {#if !loaded}
      <div class="empty">正在读取…</div>
    {:else if !shows.length && !videos.length}
      <div class="empty">还没有视频。到“设置”里添加视频文件夹并扫描，或直接把视频拖进窗口。</div>
    {:else}
      {#if shows.length}
        <div class="grp">电视剧</div>
        <div class="mp-grid">
          {#each shows as s (s.id)}
            <div class="mp-card" role="button" tabindex="0" onclick={() => (openId = s.id)} onkeydown={e => e.key === 'Enter' && (openId = s.id)} oncontextmenu={e => seriesMenu(e, s)}>
              <Poster poster={s.poster} thumb={s.thumb} name={s.title}>
                <span class="mp-badge">{seriesBadge(s)}</span>
                <span class="mp-pt">{s.title}</span>
                <i class="mp-bar" style:width="{pct(s)}%"></i>
                <MoreButton onopen={e => seriesMenu(e, s)} />
              </Poster>
              <div class="mp-card-t">{s.title}</div>
              <div class="mp-card-s">共 {s.episodes} 集{s.missing_subs ? ` · ${s.missing_subs} 集缺字幕` : ''}</div>
            </div>
          {/each}
        </div>
      {/if}

      {#if videos.length}
        <div class="grp">独立视频</div>
        <div class="mp-grid wide">
          {#each videos as v (v.id)}
            <div
              class="mp-card"
              role="button"
              tabindex="0"
              onclick={() => playVideo(v)}
              onkeydown={e => e.key === 'Enter' && playVideo(v)}
              oncontextmenu={e => videoMenu(e, v)}
              use:lazyThumb={{ id: v.id, thumb: v.thumb, onthumb: n => (thumbs[v.id] = n) }}
            >
              <Poster thumb={thumbs[v.id] ?? v.thumb} name={v.title}>
                <span class="mp-badge">{vBadge(v)}</span>
                <i class="mp-bar" style:width="{vPct(v)}%"></i>
                <MoreButton onopen={e => videoMenu(e, v)} />
              </Poster>
              <div class="mp-card-t">{v.title}</div>
              <div class="mp-card-s">{fmtT(v.duration)}{v.resolution ? ` · ${v.resolution}` : ''}</div>
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  {/if}
</section>
