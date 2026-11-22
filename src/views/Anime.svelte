<script lang="ts">
  import { untrack } from 'svelte';
  import MoreButton from '../components/MoreButton.svelte';
  import BatchBar from '../components/BatchBar.svelte';
  import SelCheck from '../components/SelCheck.svelte';
  import Icon from '../lib/Icon.svelte';
  import { openMenu, mediaEntries, removeMany, deleteMany, subtitleMany, type MenuEntry } from '../lib/items.svelte';
  import { Selection, noTextSelect } from '../lib/select.svelte';
  import Poster from '../components/media/Poster.svelte';
  import SeriesDetail from '../components/media/SeriesDetail.svelte';
  import { lib } from '../lib/library.svelte';
  import { ui } from '../lib/ui.svelte';
  import { invoke, inTauri, type Series } from '../lib/api';
  import { seriesBadge } from '../lib/media.svelte';

  type Filter = 'all' | 'on' | 'done' | 'new' | 'nosub';
  const FILTERS: [Filter, string][] = [
    ['all', '全部'],
    ['on', '在追'],
    ['done', '已看完'],
    ['new', '未看'],
    ['nosub', '缺字幕'],
  ];

  let filter = $state<Filter>('all');
  let list = $state<Series[]>([]);
  let loaded = $state(false);
  let openId = $state<number | null>(null);

  const openSeries = $derived(openId == null ? null : (list.find(s => s.id === openId) ?? null));
  const shown = $derived(list.filter(s => match(s, filter)));

  function match(s: Series, f: Filter): boolean {
    const started = s.watched > 0 || s.last_played != null;
    const done = s.episodes > 0 && s.watched >= s.episodes;
    switch (f) {
      case 'on':
        return started && !done;
      case 'done':
        return done;
      case 'new':
        return !started;
      case 'nosub':
        return s.missing_subs > 0;
      default:
        return true;
    }
  }

  const pct = (s: Series) => (s.episodes ? (s.watched / s.episodes) * 100 : 0);

  async function load() {
    if (!inTauri) {
      loaded = true;
      return;
    }
    list = await invoke<Series[]>('list_series', { kind: 'anime' });
    loaded = true;
  }
  $effect(() => {
    void lib.version;
    void ui.videoId;
    load();
  });

  // 搜索结果跳过来时直接打开那部番剧
  $effect(() => {
    const sid = ui.seriesId;
    if (sid == null || ui.view !== 'anime' || !loaded) return;
    if (list.some(s => s.id === sid)) {
      openId = sid;
      ui.seriesId = null;
    }
  });
  // ---------- 多选（按部） ----------
  const sel = new Selection();
  const order = $derived(shown.map(s => s.id));
  $effect(() => { const o = order; untrack(() => sel.prune(o)); });
  $effect(() => { if (openId != null) untrack(() => sel.clear()); });

  function clickS(e: MouseEvent, s: Series) {
    if (!sel.click(e, s.id, order)) openId = s.id;
  }
  function batch(): MenuEntry[] {
    const ids = sel.ids(order);
    const done = (ok: boolean) => ok && sel.clear();
    return [
      { label: '生成 AI 字幕', run: () => subtitleMany(list.find(s => s.id === ids[0])?.next_id ?? null, ids.length) },
      { label: '整理文件…', run: () => ui.go('org') },
      'sep',
      { label: '从媒体库移除…', run: async () => done(await removeMany('series', ids)) },
      { label: '删除文件…', danger: true, run: async () => done(await deleteMany('series', ids)) },
    ];
  }

  function seriesMenu(e: MouseEvent, s: Series) {
    if (sel.has(s.id) && sel.size > 1) return openMenu(e, `已选 ${sel.size} 部`, batch());
    openMenu(e, s.title, mediaEntries('series', s.id, s.title, [
      { label: '打开详情', run: () => (openId = s.id) },
      ...(s.next_id != null ? [{ label: '继续观看', run: () => s.next_id != null && ui.openItem(s.next_id, 'anime') }] : []),
    ]));
  }
</script>

<svelte:window onkeydown={e => openId == null && sel.keys(e, () => order)} />

<section class="lib on" class:multi={sel.on}>
  {#if openSeries}
    <SeriesDetail series={openSeries} kind="anime" onback={() => (openId = null)} onchanged={load} />
  {:else}
    <header>
      <div>
        <h2>番剧</h2>
        <p>{list.length} 部 · 点进去按集观看，缺字幕的可以生成 AI 字幕</p>
      </div>
      {#if list.length}
        <button class="mp-chip msel" class:on={sel.on} onclick={() => (sel.on ? sel.clear() : (sel.mode = true))}><Icon name="check" /> {sel.on ? '退出多选' : '多选'}</button>
      {/if}
    </header>

    <div class="mp-chips" style="margin:18px 0 16px">
      {#each FILTERS as [f, label] (f)}
        <button class="mp-chip" class:on={filter === f} onclick={() => (filter = f)}>{label}</button>
      {/each}
    </div>

    {#if sel.on}
      <BatchBar {sel} unit="部" total={order.length} onall={() => sel.selectAll(order)} entries={batch()} />
    {/if}

    {#if !loaded}
      <div class="empty">正在读取…</div>
    {:else if !list.length}
      <div class="empty">还没有番剧。到“设置”里添加番剧文件夹并扫描，或直接把视频拖进窗口。</div>
    {:else if !shown.length}
      <div class="empty">没有符合条件的番剧</div>
    {:else}
      <div class="mp-grid">
        {#each shown as s (s.id)}
          <div class="mp-card" class:picked={sel.has(s.id)} role="button" tabindex="0" onclick={e => clickS(e, s)} onmousedown={noTextSelect} onkeydown={e => e.key === 'Enter' && (openId = s.id)} oncontextmenu={e => seriesMenu(e, s)}>
            <Poster poster={s.poster} thumb={s.thumb} name={s.title}>
              <SelCheck card checked={sel.has(s.id)} onclick={e => (e.shiftKey ? sel.range(order, s.id, true) : sel.toggle(s.id))} />
              <span class="mp-badge">{seriesBadge(s)}</span>
              <span class="mp-pt">{s.title}</span>
              <i class="mp-bar" style:width="{pct(s)}%"></i>
              <MoreButton onopen={e => seriesMenu(e, s)} />
            </Poster>
            <div class="mp-card-t">{s.title}</div>
            <div class="mp-card-s">
              {s.seasons > 1 ? `${s.seasons} 季 · ` : ''}共 {s.episodes} 集{s.missing_subs ? ` · ${s.missing_subs} 集缺字幕` : ''}
            </div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</section>

<style>
  .msel { display: inline-flex; align-items: center; gap: 5px; }
</style>
