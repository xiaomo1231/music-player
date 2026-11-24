<script lang="ts">
  import { untrack } from 'svelte';
  import MoreButton from '../components/MoreButton.svelte';
  import BatchBar from '../components/BatchBar.svelte';
  import SelCheck from '../components/SelCheck.svelte';
  import Icon from '../lib/Icon.svelte';
  import { openMenu, mediaEntries, removeMany, deleteMany, type MenuEntry } from '../lib/items.svelte';
  import { Selection, noTextSelect } from '../lib/select.svelte';
  import { invoke, inTauri, type ComicItem } from '../lib/api';
  import { lib } from '../lib/library.svelte';
  import { ui, toast } from '../lib/ui.svelte';
  import Cover from '../components/Cover.svelte';
  import ComicExportModal from '../components/em/ComicExportModal.svelte';
  import CoverPicker from '../components/media/CoverPicker.svelte';
  import WorkDetail from '../components/shelf/WorkDetail.svelte';
  import FollowStrip from '../components/shelf/FollowStrip.svelte';
  import { art, artPoster, artPending, artUi, loadArt, openCoverPicker } from '../lib/media.svelte';
  import { comicDone, comicVol, comicFmt, nextVol, unitOf, type Vol } from '../lib/shelf';

  /** 一部漫画：同一系列的所有话 / 卷；没有系列名的单本自成一部 */
  interface Work {
    key: string;
    name: string;
    /** 有系列名（可以选官方海报） */
    named: boolean;
    items: ComicItem[];
    unit: '话' | '卷';
    read: number;
    started: boolean;
    lastRead: number;
    next: ComicItem | null;
  }

  type Filter = 'all' | 'on' | 'done' | 'new';
  const FILTERS: [Filter, string][] = [
    ['all', '全部'],
    ['on', '在读'],
    ['done', '已读完'],
    ['new', '未读'],
  ];
  type Sort = 'recent' | 'name' | 'added';

  let comics = $state<ComicItem[]>([]);
  let loaded = $state(false);
  let filter = $state<Filter>('all');
  let sort = $state<Sort>('recent');
  let openKey = $state<string | null>(null);
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

  // 媒体库变化、关掉阅读器（进度变了）时重新读取
  $effect(() => {
    void lib.version;
    void ui.comicId;
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

  /** 按系列折叠（后端已按系列、卷名自然排序） */
  const works = $derived.by<Work[]>(() => {
    const m = new Map<string, ComicItem[]>();
    for (const c of comics) {
      const k = c.series ? 's:' + c.series : '#' + c.id;
      const arr = m.get(k);
      if (arr) arr.push(c);
      else m.set(k, [c]);
    }
    return [...m.entries()].map(([key, items]) => {
      const named = key.startsWith('s:');
      return {
        key,
        name: named ? items[0].series : items[0].title,
        named,
        items,
        unit: unitOf(items.map(c => c.title)),
        read: items.filter(comicDone).length,
        started: items.some(c => c.last_read != null),
        lastRead: Math.max(0, ...items.map(c => c.last_read ?? 0)),
        next: nextVol(items, comicDone, c => c.last_read),
      };
    });
  });

  const shown = $derived.by(() => {
    const list = works.filter(w => match(w, filter));
    if (sort === 'name') return list.sort((a, b) => a.name.localeCompare(b.name, 'zh-CN', { numeric: true }));
    if (sort === 'added') return list.sort((a, b) => Math.max(...b.items.map(c => c.id)) - Math.max(...a.items.map(c => c.id)));
    return list.sort((a, b) => b.lastRead - a.lastRead || a.name.localeCompare(b.name, 'zh-CN', { numeric: true }));
  });
  const openWork = $derived(openKey == null ? null : (works.find(w => w.key === openKey) ?? null));
  // 整部都被移除 / 删除了：回到书架
  $effect(() => {
    if (openKey != null && loaded && !openWork) untrack(() => (openKey = null));
  });

  function match(w: Work, f: Filter): boolean {
    const done = w.read >= w.items.length;
    switch (f) {
      case 'on':
        return w.started && !done;
      case 'done':
        return done;
      case 'new':
        return !w.started;
      default:
        return true;
    }
  }

  const posterOf = (w: Work): string | null => (w.named ? artPoster(art.comicByName[w.name]) : null) ?? w.items[0].cover;
  const pctOf = (w: Work) => (w.items.length ? (w.read / w.items.length) * 100 : 0);

  function subline(w: Work): string {
    if (w.items.length === 1) {
      const c = w.items[0];
      return comicDone(c) ? '已读完' : c.last_read == null ? '未读' : `第 ${c.page + 1} / ${c.pages} 页`;
    }
    const left = w.items.length - w.read;
    if (!left) return `共 ${w.items.length} ${w.unit} · 已读完`;
    if (!w.started) return `共 ${w.items.length} ${w.unit} · 未读`;
    return `读到 ${w.next?.title ?? ''} · ${left} ${w.unit}未读`;
  }

  function read(id: number) {
    ui.comicId = id;
  }
  /** 点作品：单本直接打开，多话进详情 */
  function openW(w: Work) {
    if (w.items.length === 1) read(w.items[0].id);
    else openKey = w.key;
  }
  function resume(w: Work) {
    read((w.next ?? w.items[0]).id);
  }

  async function mark(ids: number[], readFlag: boolean) {
    if (!ids.length || !inTauri) return;
    try {
      await invoke('set_comics_read', { ids, read: readFlag });
      toast(readFlag ? `已标记 ${ids.length} 卷为已读` : `已标记 ${ids.length} 卷为未读`);
      await load();
    } catch (e) {
      toast(`标记失败：${e}`);
    }
  }

  // ---------- 书架多选（按部） ----------
  const sel = new Selection();
  // 每部用第一卷的 id 当选择 id
  const wid = (w: Work) => w.items[0].id;
  const order = $derived(shown.map(wid));
  const byId = $derived(new Map(shown.map(w => [wid(w), w])));
  $effect(() => { const o = order; untrack(() => sel.prune(o)); });
  $effect(() => { if (openKey != null) untrack(() => sel.clear()); });

  function idsOfSel(): number[] {
    return sel.ids(order).flatMap(i => byId.get(i)?.items.map(c => c.id) ?? []);
  }
  function batch(): MenuEntry[] {
    const ids = idsOfSel();
    const done = (ok: boolean) => ok && sel.clear();
    return [
      { label: '标记为已读', run: async () => { await mark(ids, true); sel.clear(); } },
      { label: '标记为未读', run: async () => { await mark(ids, false); sel.clear(); } },
      { label: '按规则导出…', run: () => exportByRule(ids) },
      'sep',
      { label: '从媒体库移除…', run: async () => done(await removeMany('comic', ids)) },
      { label: '删除文件…', danger: true, run: async () => done(await deleteMany('comic', ids)) },
    ];
  }
  function clickW(e: MouseEvent, w: Work) {
    if (!sel.click(e, wid(w), order)) openW(w);
  }

  function workEntries(w: Work): MenuEntry[] {
    const ids = w.items.map(c => c.id);
    const all = w.read >= w.items.length;
    return [
      ...(w.items.length > 1 ? [{ label: '打开详情', run: () => (openKey = w.key) }] : []),
      { label: w.started && !all ? '继续阅读' : '开始阅读', run: () => resume(w) },
      ...(w.named ? [{ label: '选择官方海报…', run: () => pickSeries(w.name) }] : []),
      all ? { label: '全部标记未读', run: () => mark(ids, false) } : { label: '全部标记已读', run: () => mark(ids, true) },
      { label: '按规则导出…', run: () => exportByRule(ids) },
      'sep',
      { label: '从媒体库移除整部…', run: () => removeMany('comic', ids) },
      { label: '删除整部的文件…', danger: true, run: () => deleteMany('comic', ids) },
    ];
  }
  function workMenu(e: MouseEvent, w: Work) {
    if (sel.has(wid(w)) && sel.size > 1) return openMenu(e, `已选 ${sel.size} 部`, batch());
    if (w.items.length === 1) {
      const c = w.items[0];
      return openMenu(e, c.title, mediaEntries('comic', c.id, c.title, [
        { label: '打开', run: () => read(c.id) },
        comicDone(c) ? { label: '标记为未读', run: () => mark([c.id], false) } : { label: '标记为已读', run: () => mark([c.id], true) },
        ...(w.named ? [{ label: '选择官方海报…', run: () => pickSeries(w.name) }] : []),
      ]));
    }
    openMenu(e, w.name, workEntries(w));
  }

  // ---------- 详情页里的单卷菜单 ----------
  function volMenu(e: MouseEvent, v: Vol, s: Selection, entries: () => MenuEntry[]) {
    if (s.has(v.id) && s.size > 1) return openMenu(e, `已选 ${s.size} 卷`, entries());
    openMenu(e, v.title, mediaEntries('comic', v.id, v.title, [
      { label: '打开', run: () => read(v.id) },
      v.done ? { label: '标记为未读', run: () => mark([v.id], false) } : { label: '标记为已读', run: () => mark([v.id], true) },
    ]));
  }
  function volBatch(ids: number[], done: (ok: boolean) => void): MenuEntry[] {
    return [
      { label: '按规则导出…', run: () => exportByRule(ids) },
      'sep',
      { label: '从媒体库移除…', run: async () => done(await removeMany('comic', ids)) },
      { label: '删除文件…', danger: true, run: async () => done(await deleteMany('comic', ids)) },
    ];
  }
</script>

<svelte:window onkeydown={e => openKey == null && sel.keys(e, () => order)} />

<section class="lib on" class:multi={sel.on}>
  {#if openWork}
    <WorkDetail
      title={openWork.name}
      poster={posterOf(openWork)}
      kind="漫画"
      unit={openWork.unit}
      vols={openWork.items.map(comicVol)}
      nextId={openWork.next?.id ?? null}
      onback={() => (openKey = null)}
      onopen={read}
      onmenu={volMenu}
      batch={volBatch}
      onmark={mark}
      onheadmenu={e => openMenu(e, openWork.name, workEntries(openWork).filter(x => x === 'sep' || x.label !== '打开详情'))}
      onposter={openWork.named ? () => pickSeries(openWork.name) : undefined}
    />
  {:else}
    <header>
      <div>
        <h2>漫画</h2>
        <p>{comics.length ? `${works.length} 部 · 共 ${comics.length} 卷 · 点进去按话 / 卷阅读` : '本地 CBZ / ZIP / PDF / 图片文件夹'}</p>
      </div>
      {#if works.length}
        <button class="mp-chip msel" class:on={sel.on} onclick={() => (sel.on ? sel.clear() : (sel.mode = true))}><Icon name="check" /> {sel.on ? '退出多选' : '多选'}</button>
      {/if}
    </header>

    {#if works.length}
      <div class="mp-chips bar">
        {#each FILTERS as [f, label] (f)}
          <button class="mp-chip" class:on={filter === f} onclick={() => (filter = f)}>{label}</button>
        {/each}
        <span class="grow"></span>
        <select class="mp-chip" bind:value={sort} aria-label="排序">
          <option value="recent">最近阅读</option>
          <option value="name">作品名</option>
          <option value="added">最近添加</option>
        </select>
      </div>
    {/if}

    <FollowStrip kind="manga" names={works.map(w => w.name)} />

    {#if sel.on}
      <BatchBar {sel} unit="部" total={order.length} onall={() => sel.selectAll(order)} entries={batch()} />
    {/if}

    {#if !loaded}
      <p class="empty">正在读取…</p>
    {:else if !comics.length}
      <p class="empty">{inTauri ? '还没有漫画。把 CBZ、ZIP、PDF 文件或图片文件夹拖进窗口，或者添加媒体库目录。' : '在桌面程序中查看漫画书架。'}</p>
    {:else if !shown.length}
      <p class="empty">没有符合条件的漫画</p>
    {:else}
      <div class="mp-grid">
        {#each shown as w (w.key)}
          {@const left = w.items.length - w.read}
          {@const i = wid(w)}
          <div class="mp-card mp-book" class:picked={sel.has(i)} role="button" tabindex="0" onclick={e => clickW(e, w)} onmousedown={noTextSelect} onkeydown={e => e.key === 'Enter' && openW(w)} oncontextmenu={e => workMenu(e, w)}>
            <Cover cover={posterOf(w)} name={w.name} tag="div" cls="mp-poster">
              <SelCheck card checked={sel.has(i)} onclick={e => (e.shiftKey ? sel.range(order, i, true) : sel.toggle(i))} />
              <span class="mp-badge">{w.items.length > 1 ? `${w.items.length} ${w.unit}` : comicFmt(w.items[0])}</span>
              {#if w.named && artPending(art.comicByName[w.name])}<span class="mp-artq">海报待确认</span>{/if}
              {#if w.items.length > 1 && w.started && left > 0}<span class="unread">{left} 未读</span>{/if}
              {#if !posterOf(w)}<span class="mp-pt">{w.name}</span>{/if}
              <i class="mp-bar" style:width="{w.items.length > 1 ? pctOf(w) : comicVol(w.items[0]).pct}%"></i>
              <MoreButton onopen={e => workMenu(e, w)} />
            </Cover>
            <div class="mp-card-t">{w.name}</div>
            <div class="mp-card-s">{subline(w)}</div>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</section>

{#if exportIds}
  <ComicExportModal ids={exportIds} onclose={() => (exportIds = null)} ondone={r => r.outputs.length && sel.clear()} />
{/if}
{#if artUi.picker}
  <CoverPicker {...artUi.picker} onclose={() => (artUi.picker = null)} />
{/if}

<style>
  .msel { display: inline-flex; align-items: center; gap: 5px; }
  .bar { display: flex; align-items: center; gap: 6px; margin: 18px 0 16px; }
  .grow { flex: 1; }
  select.mp-chip { color: inherit; font: inherit; font-size: 12.5px; }
  select.mp-chip option { color: #000; }
  .unread { position: absolute; right: 8px; top: 8px; font-size: 10px; padding: 2px 6px; border-radius: 5px; background: var(--mp-accent); color: #fff; }
  .mp-card {
    background: none;
    border: 0;
    padding: 0;
    color: inherit;
    font: inherit;
    text-align: left;
  }
  .empty {
    color: var(--mp-sub);
    padding: 40px 0;
  }
</style>
