<script lang="ts">
  import { untrack } from 'svelte';
  import MoreButton from '../components/MoreButton.svelte';
  import BatchBar from '../components/BatchBar.svelte';
  import SelCheck from '../components/SelCheck.svelte';
  import Icon from '../lib/Icon.svelte';
  import { openMenu, mediaEntries, removeMany, deleteMany, type MenuEntry } from '../lib/items.svelte';
  import { Selection, noTextSelect } from '../lib/select.svelte';
  import { invoke, inTauri, fileUrl, type ComicItem } from '../lib/api';
  import { lib } from '../lib/library.svelte';
  import { ui, toast } from '../lib/ui.svelte';
  import Cover from '../components/Cover.svelte';
  import ComicExportModal from '../components/em/ComicExportModal.svelte';
  import CoverPicker from '../components/media/CoverPicker.svelte';
  import { art, artPoster, artPending, artUi, loadArt, openCoverPicker } from '../lib/media.svelte';

  let comics = $state<ComicItem[]>([]);
  let loaded = $state(false);
  /** 按规则导出：选中的漫画 */
  let exportIds = $state.raw<number[] | null>(null);
  function exportByRule(ids: number[]) {
    const v = ids.filter(id => comics.find(c => c.id === id)?.format !== 'pdf');
    if (v.length) exportIds = v;
    else toast('PDF 不能按规则导出');
  }

  async function pickSeries(name: string) {
    const id = art.comicByName[name]?.id ?? (await invoke<number>('art_comic_id', { series: name }));
    openCoverPicker('comic', id, name);
  }

  $effect(() => {
    void lib.version;
    load();
  });

  async function load() {
    if (!inTauri) {
      loaded = true;
      return;
    }
    try {
      comics = await invoke<ComicItem[]>('list_comics');
      loadArt();
    } catch {
      comics = [];
    }
    loaded = true;
  }

  /** 按系列分组（后端已按系列、卷名自然排序） */
  const groups = $derived.by(() => {
    const m = new Map<string, ComicItem[]>();
    for (const c of comics) {
      const k = c.series || '未分类';
      const arr = m.get(k);
      if (arr) arr.push(c);
      else m.set(k, [c]);
    }
    return [...m.entries()].map(([name, items]) => ({ name, items }));
  });

  function fmt(c: ComicItem): string {
    if (c.format === 'pdf') return 'PDF';
    if (c.format === 'folder') return '图片';
    return c.format.toUpperCase();
  }

  function pct(c: ComicItem): number {
    return c.pages > 0 ? Math.min(100, ((c.page + 1) / c.pages) * 100) : 0;
  }

  function status(c: ComicItem): string {
    if (c.last_read == null) return '未读';
    if (c.pages <= 0) return '已读过';
    if (c.page >= c.pages - 1) return '已读完';
    return `第 ${c.page + 1} / ${c.pages} 页`;
  }

  function open(id: number) {
    ui.comicId = id;
  }
  // ---------- 多选 ----------
  const sel = new Selection();
  const order = $derived(comics.map(x => x.id));
  $effect(() => { const o = order; untrack(() => sel.prune(o)); });
  function clickItem(e: MouseEvent, id: number) {
    if (!sel.click(e, id, order)) open(id);
  }
  function batch(): MenuEntry[] {
    const ids = sel.ids(order);
    const done = (ok: boolean) => ok && sel.clear();
    return [
      { label: '按规则导出…', run: () => exportByRule(ids) },
      'sep',
      { label: '从媒体库移除…', run: async () => done(await removeMany('comic', ids)) },
      { label: '删除文件…', danger: true, run: async () => done(await deleteMany('comic', ids)) },
    ];
  }

  function comicMenu(e: MouseEvent, c: ComicItem) {
    if (sel.has(c.id) && sel.size > 1) return openMenu(e, `已选 ${sel.size} 卷`, batch());
    openMenu(e, c.title, mediaEntries('comic', c.id, c.title, [{ label: '打开', run: () => open(c.id) }]));
  }
</script>

<svelte:window onkeydown={e => sel.keys(e, () => order)} />

<section class="lib on" class:multi={sel.on}>
  <header>
    <div>
      <h2>漫画</h2>
      <p>{comics.length ? `${groups.length} 个系列 · 共 ${comics.length} 卷` : '本地 CBZ / ZIP / PDF / 图片文件夹'}</p>
    </div>
    {#if comics.length}
      <button class="mp-chip msel" class:on={sel.on} onclick={() => (sel.on ? sel.clear() : (sel.mode = true))}><Icon name="check" /> {sel.on ? '退出多选' : '多选'}</button>
    {/if}
  </header>

  {#if sel.on}
    <BatchBar {sel} unit="卷" total={order.length} onall={() => sel.selectAll(order)} entries={batch()} />
  {/if}

  {#if !loaded}
    <p class="empty">正在读取…</p>
  {:else if !comics.length}
    <p class="empty">{inTauri ? '还没有漫画。把 CBZ、ZIP、PDF 文件或图片文件夹拖进窗口，或者添加媒体库目录。' : '在桌面程序中查看漫画书架。'}</p>
  {:else}
    {#each groups as g (g.name)}
      {@const sp = g.name !== '未分类' ? artPoster(art.comicByName[g.name]) : null}
      <h3 class="series">
        {#if sp}<button class="cs-poster" style:background-image={`url("${fileUrl(sp)}")`} title="更换系列海报" onclick={() => pickSeries(g.name)} aria-label="更换系列海报"></button>{/if}
        {g.name} <small>{g.items.length} 卷</small>
        {#if g.name !== '未分类'}
          <button class="mp-chip cs-pick" onclick={() => pickSeries(g.name)}>{artPending(art.comicByName[g.name]) ? '海报待确认' : sp ? '更换海报' : '选择海报'}</button>
        {/if}
      </h3>
      <div class="mp-grid">
        {#each g.items as c (c.id)}
          <div class="mp-card mp-book" class:picked={sel.has(c.id)} role="button" tabindex="0" onclick={e => clickItem(e, c.id)} onmousedown={noTextSelect} onkeydown={e => e.key === 'Enter' && open(c.id)} oncontextmenu={e => comicMenu(e, c)}>
            <Cover cover={c.cover ?? (c.series ? artPoster(art.comicByName[c.series]) : null)} name={c.title} tag="div" cls="mp-poster">
            <SelCheck card checked={sel.has(c.id)} onclick={e => (e.shiftKey ? sel.range(order, c.id, true) : sel.toggle(c.id))} />
              <span class="mp-badge">{fmt(c)}</span>
              {#if !c.cover}<span class="mp-pt">{c.title}</span>{/if}
              <i class="mp-bar" style="width:{pct(c)}%"></i>
              <MoreButton onopen={e => comicMenu(e, c)} />
            </Cover>
            <div class="mp-card-t">{c.title}</div>
            <div class="mp-card-s">{status(c)}</div>
          </div>
        {/each}
      </div>
    {/each}
  {/if}
</section>

{#if exportIds}
  <ComicExportModal ids={exportIds} onclose={() => (exportIds = null)} ondone={r => r.outputs.length && sel.clear()} />
{/if}
{#if artUi.picker}
  <CoverPicker {...artUi.picker} onclose={() => (artUi.picker = null)} />
{/if}

<style>
  .series { display: flex; align-items: center; gap: 10px; }
  .cs-poster { all: unset; cursor: pointer; width: 30px; aspect-ratio: 2 / 3; border-radius: 5px; background: center / cover no-repeat; box-shadow: 0 2px 8px rgba(0,0,0,.3); }
  .cs-pick { margin-left: auto; font-size: 12px; padding: 3px 10px; }
  .msel { display: inline-flex; align-items: center; gap: 5px; }
  .mp-card {
    background: none;
    border: 0;
    padding: 0;
    color: inherit;
    font: inherit;
    text-align: left;
  }
  .series {
    margin: 26px 0 12px;
    font-size: 16px;
    font-weight: 600;
  }
  .series small {
    color: var(--mp-sub);
    font-size: 12px;
    font-weight: 400;
  }
  .empty {
    color: var(--mp-sub);
    padding: 40px 0;
  }
</style>
