<script lang="ts">
  import { untrack } from 'svelte';
  import MoreButton from '../components/MoreButton.svelte';
  import BatchBar from '../components/BatchBar.svelte';
  import SelCheck from '../components/SelCheck.svelte';
  import Icon from '../lib/Icon.svelte';
  import { openMenu, mediaEntries, removeMany, deleteMany, subtitleMany, type MenuEntry } from '../lib/items.svelte';
  import { Selection, claim, noTextSelect } from '../lib/select.svelte';
  import Poster from '../components/media/Poster.svelte';
  import SeriesDetail from '../components/media/SeriesDetail.svelte';
  import { lib } from '../lib/library.svelte';
  import { ui } from '../lib/ui.svelte';
  import { invoke, inTauri, type Series, type Video } from '../lib/api';
  import { fmtT } from '../lib/util';
  import { lazyThumb, seriesBadge, lazySeriesCover, type SeriesCover } from '../lib/media.svelte';

  // movies：“电影”分类页，只列电影；否则是“视频”页（电视剧 + 独立视频）
  let { movies = false }: { movies?: boolean } = $props();
  const unit = $derived(movies ? '部电影' : '个视频');

  let videos = $state<Video[]>([]);
  let shows = $state<Series[]>([]);
  let loaded = $state(false);
  /** 刚找到 / 生成的封面（不用整页重新读取） */
  let covers = $state<Record<number, SeriesCover>>({});
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
    const [v, s] = await Promise.all([invoke<Video[]>('list_videos'), movies ? Promise.resolve([]) : invoke<Series[]>('list_series', { kind: 'tv' })]);
    videos = v.filter(x => !!x.movie === movies);
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
    if (sid == null || movies || ui.view !== 'video' || !loaded) return;
    if (shows.some(s => s.id === sid)) {
      openId = sid;
      ui.seriesId = null;
    }
  });

  // 独立视频：播放时列表就是当前海报墙的顺序
  function playVideo(v: Video) {
    ui.openItem(v.id, 'video', videos.map(x => x.id));
  }
  // ---------- 多选：电视剧和独立视频各一组，同一时间只在一组里选 ----------
  const selS = new Selection();
  const selV = new Selection();
  const sOrder = $derived(shows.map(s => s.id));
  const vOrder = $derived(videos.map(v => v.id));
  $effect(() => { const o = sOrder; untrack(() => selS.prune(o)); });
  $effect(() => { const o = vOrder; untrack(() => selV.prune(o)); });
  $effect(() => { if (openId != null) untrack(() => { selS.clear(); selV.clear(); }); });

  function clickS(e: MouseEvent, s: Series) {
    claim(selS, selV);
    if (!selS.click(e, s.id, sOrder)) openId = s.id;
  }
  function clickV(e: MouseEvent, v: Video) {
    claim(selV, selS);
    if (!selV.click(e, v.id, vOrder)) playVideo(v);
  }
  function seriesBatch(): MenuEntry[] {
    const ids = selS.ids(sOrder);
    const done = (ok: boolean) => ok && selS.clear();
    return [
      { label: '生成 AI 字幕', run: () => subtitleMany(shows.find(s => s.id === ids[0])?.next_id ?? null, ids.length) },
      { label: '整理文件…', run: () => ui.go('org') },
      'sep',
      { label: '从媒体库移除…', run: async () => done(await removeMany('series', ids)) },
      { label: '删除文件…', danger: true, run: async () => done(await deleteMany('series', ids)) },
    ];
  }
  function videoBatch(): MenuEntry[] {
    const ids = selV.ids(vOrder);
    const vs = ids.map(id => videos.find(v => v.id === id)!).filter(Boolean);
    const done = (ok: boolean) => ok && selV.clear();
    return [
      { label: '播放', run: () => ui.openItem(ids[0], 'video', ids) },
      { label: '生成 AI 字幕', run: () => subtitleMany(ids[0], ids.length) },
      { label: '转换格式…', run: () => { ui.convertPaths = vs.map(v => v.path); ui.go('convert'); } },
      { label: '整理文件…', run: () => ui.go('org') },
      'sep',
      { label: '从媒体库移除…', run: async () => done(await removeMany('video', ids)) },
      { label: '删除文件…', danger: true, run: async () => done(await deleteMany('video', ids)) },
    ];
  }
  function onKeys(e: KeyboardEvent) {
    if (openId != null) return;
    if (selS.size) selS.keys(e, () => sOrder);
    else if (vOrder.length) selV.keys(e, () => vOrder);
    else selS.keys(e, () => sOrder);
  }

  function seriesMenu(e: MouseEvent, s: Series) {
    if (selS.has(s.id) && selS.size > 1) return openMenu(e, `已选 ${selS.size} 部`, seriesBatch());
    openMenu(e, s.title, mediaEntries('series', s.id, s.title, [
      { label: '打开详情', run: () => (openId = s.id) },
      ...(s.next_id != null ? [{ label: '继续观看', run: () => s.next_id != null && ui.openItem(s.next_id, 'video') }] : []),
    ]));
  }
  function videoMenu(e: MouseEvent, v: Video) {
    if (selV.has(v.id) && selV.size > 1) return openMenu(e, `已选 ${selV.size} ${unit}`, videoBatch());
    openMenu(e, v.title, mediaEntries('video', v.id, v.title, [
      { label: '播放', run: () => playVideo(v) },
      { label: '生成 AI 字幕', run: () => { ui.subVideo = v.id; ui.go('aisub'); } },
      { label: '转换格式…', run: () => { ui.convertPaths = [v.path]; ui.go('convert'); } },
      { label: '整理文件…', run: () => ui.go('org') },
    ]));
  }
</script>

<svelte:window onkeydown={onKeys} />

<section class="lib on" class:multi={selS.on || selV.on}>
  {#if openSeries}
    <SeriesDetail series={openSeries} kind="tv" onback={() => (openId = null)} onchanged={load} />
  {:else}
    <header>
      <div>
        <h2>{movies ? '电影' : '视频'}</h2>
        <p>{movies ? `${videos.length} 部电影` : `${shows.length} 部电视剧 · ${videos.length} 个独立视频`}</p>
      </div>
      {#if shows.length || videos.length}
        <button class="mp-chip msel" class:on={selS.on || selV.on} onclick={() => (selS.on || selV.on ? (selS.clear(), selV.clear()) : ((videos.length ? selV : selS).mode = true))}
          ><Icon name="check" /> {selS.on || selV.on ? '退出多选' : '多选'}</button>
      {/if}
    </header>

    {#if selS.on}
      <BatchBar sel={selS} unit="部" total={sOrder.length} onall={() => selS.selectAll(sOrder)} entries={seriesBatch()} />
    {:else if selV.on}
      <BatchBar sel={selV} {unit} total={vOrder.length} onall={() => selV.selectAll(vOrder)} entries={videoBatch()}
        hint={movies ? '单击勾选，Shift 连选，Ctrl+A 全选' : '单击勾选，Shift 连选，Ctrl+A 全选；点电视剧海报可以改选电视剧'} />
    {/if}

    {#if !loaded}
      <div class="empty">正在读取…</div>
    {:else if !shows.length && !videos.length}
      <div class="empty">
        {movies
          ? '还没有电影。放在名字带“电影”或 Movies 的文件夹里的视频，以及“片名.2019.1080p.BluRay”这种发布名的长视频会归到这里。'
          : '还没有视频。到“设置”里添加视频文件夹并扫描，或直接把视频拖进窗口。'}
      </div>
    {:else}
      {#if shows.length}
        <div class="grp">电视剧</div>
        <div class="mp-grid">
          {#each shows as s (s.id)}
            <div class="mp-card" class:picked={selS.has(s.id)} role="button" tabindex="0" onclick={e => clickS(e, s)} onmousedown={noTextSelect} onkeydown={e => e.key === 'Enter' && (openId = s.id)} oncontextmenu={e => seriesMenu(e, s)} use:lazySeriesCover={{ id: s.id, has: !!(s.poster || s.frame), oncover: c => (covers[s.id] = c) }}>
              <Poster poster={covers[s.id]?.poster ?? s.poster} frame={covers[s.id]?.frame ?? s.frame} thumb={s.thumb} name={s.title}>
                <SelCheck card checked={selS.has(s.id)} onclick={e => { claim(selS, selV); if (e.shiftKey) selS.range(sOrder, s.id, true); else selS.toggle(s.id); }} />
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
        {#if !movies}<div class="grp">独立视频</div>{/if}
        <div class="mp-grid wide">
          {#each videos as v (v.id)}
            <div
              class="mp-card"
              class:picked={selV.has(v.id)}
              role="button"
              tabindex="0"
              onclick={e => clickV(e, v)}
              onmousedown={noTextSelect}
              onkeydown={e => e.key === 'Enter' && playVideo(v)}
              oncontextmenu={e => videoMenu(e, v)}
              use:lazyThumb={{ id: v.id, thumb: v.thumb, onthumb: n => (thumbs[v.id] = n) }}
            >
              <Poster thumb={thumbs[v.id] ?? v.thumb} name={v.title}>
                <SelCheck card checked={selV.has(v.id)} onclick={e => { claim(selV, selS); if (e.shiftKey) selV.range(vOrder, v.id, true); else selV.toggle(v.id); }} />
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

<style>
  .msel { display: inline-flex; align-items: center; gap: 5px; }
</style>
