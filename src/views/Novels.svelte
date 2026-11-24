<script lang="ts">
  import { untrack } from 'svelte';
  import MoreButton from '../components/MoreButton.svelte';
  import BatchBar from '../components/BatchBar.svelte';
  import SelCheck from '../components/SelCheck.svelte';
  import Icon from '../lib/Icon.svelte';
  import { openMenu, mediaEntries, removeMany, deleteMany, type MenuEntry } from '../lib/items.svelte';
  import { Selection, noTextSelect } from '../lib/select.svelte';
  import { invoke, inTauri, type BookItem } from '../lib/api';
  import { lib } from '../lib/library.svelte';
  import { ui } from '../lib/ui.svelte';
  import Cover from '../components/Cover.svelte';
  import WorkDetail from '../components/shelf/WorkDetail.svelte';
  import FollowStrip from '../components/shelf/FollowStrip.svelte';
  import { bookSeriesKey, bookProgress, bookVol, nextVol, naturalCmp, type Vol } from '../lib/shelf';

  /** 一部小说：按卷拆成多个文件的合在一起，单本自成一部 */
  interface Work {
    key: string;
    name: string;
    author: string;
    items: BookItem[];
    lastRead: number;
  }

  let books = $state<BookItem[]>([]);
  let loaded = $state(false);

  let openKey = $state<string | null>(null);

  $effect(() => {
    void lib.version;
    void ui.bookId;
    load();
  });

  async function load() {
    if (!inTauri) {
      loaded = true;
      return;
    }
    try {
      const list = await invoke<BookItem[]>('list_books');
      // 最近阅读排在前面；没读过的按书名
      books = [...list].sort((a, b) => (b.last_read ?? 0) - (a.last_read ?? 0) || a.title.localeCompare(b.title, 'zh-CN'));
    } catch {
      books = [];
    }
    loaded = true;
  }

  const progress = bookProgress;
  const isDone = (b: BookItem) => bookProgress(b).done;

  /** 同一作者、去掉卷号后同名的两本以上合成一部 */
  const works = $derived.by<Work[]>(() => {
    const m = new Map<string, BookItem[]>();
    for (const b of books) {
      const k = bookSeriesKey(b.title) + '\u0000' + (b.author || '');
      const arr = m.get(k);
      if (arr) arr.push(b);
      else m.set(k, [b]);
    }
    const out: Work[] = [];
    for (const [k, items] of m) {
      if (items.length < 2) {
        for (const b of items) out.push({ key: '#' + b.id, name: b.title, author: b.author, items: [b], lastRead: b.last_read ?? 0 });
        continue;
      }
      items.sort((a, b) => naturalCmp(a.title, b.title));
      out.push({ key: k, name: bookSeriesKey(items[0].title), author: items[0].author, items, lastRead: Math.max(0, ...items.map(b => b.last_read ?? 0)) });
    }
    // 最近阅读排在前面；没读过的按书名
    return out.sort((a, b) => b.lastRead - a.lastRead || a.name.localeCompare(b.name, 'zh-CN'));
  });
  const openWork = $derived(openKey == null ? null : (works.find(w => w.key === openKey) ?? null));
  $effect(() => {
    if (openKey != null && loaded && !openWork) untrack(() => (openKey = null));
  });
  const nextOf = (w: Work) => nextVol(w.items, isDone, b => b.last_read);
  function workLine(w: Work): string {
    if (w.items.length === 1) return `${w.author || '佚名'} · ${progress(w.items[0]).label}`;
    const read = w.items.filter(isDone).length;
    const left = w.items.length - read;
    const started = w.items.some(b => b.last_read != null);
    return `${w.author || '佚名'} · 共 ${w.items.length} 卷 · ${!left ? '已读完' : !started ? '未读' : `${left} 卷未读`}`;
  }
  function workPct(w: Work): number {
    if (w.items.length === 1) return progress(w.items[0]).pct;
    return (w.items.filter(isDone).length / w.items.length) * 100;
  }
  function openW(w: Work) {
    if (w.items.length === 1) open(w.items[0].id);
    else openKey = w.key;
  }

  function open(id: number) {
    ui.bookId = id;
  }
  // ---------- 多选 ----------
  const sel = new Selection();
  const wid = (w: Work) => w.items[0].id;
  const order = $derived(works.map(wid));
  const byId = $derived(new Map(works.map(w => [wid(w), w])));
  $effect(() => { if (openKey != null) untrack(() => sel.clear()); });
  $effect(() => { const o = order; untrack(() => sel.prune(o)); });
  function clickW(e: MouseEvent, w: Work) {
    if (!sel.click(e, wid(w), order)) openW(w);
  }
  function batch(): MenuEntry[] {
    const ids = sel.ids(order).flatMap(i => byId.get(i)?.items.map(b => b.id) ?? []);
    const done = (ok: boolean) => ok && sel.clear();
    return [
      { label: '从媒体库移除…', run: async () => done(await removeMany('book', ids)) },
      { label: '删除文件…', danger: true, run: async () => done(await deleteMany('book', ids)) },
    ];
  }

  function workMenu(e: MouseEvent, w: Work) {
    if (sel.has(wid(w)) && sel.size > 1) return openMenu(e, `已选 ${sel.size} 部`, batch());
    if (w.items.length === 1) {
      const b = w.items[0];
      return openMenu(e, b.title, mediaEntries('book', b.id, b.title, [{ label: '打开', run: () => open(b.id) }]));
    }
    openMenu(e, w.name, workEntries(w));
  }
  function workEntries(w: Work): MenuEntry[] {
    const ids = w.items.map(b => b.id);
    return [
      { label: '打开详情', run: () => (openKey = w.key) },
      { label: '继续阅读', run: () => open((nextOf(w) ?? w.items[0]).id) },
      'sep',
      { label: '从媒体库移除整部…', run: () => removeMany('book', ids) },
      { label: '删除整部的文件…', danger: true, run: () => deleteMany('book', ids) },
    ];
  }
  function volMenu(e: MouseEvent, v: Vol, s: Selection, entries: () => MenuEntry[]) {
    if (s.has(v.id) && s.size > 1) return openMenu(e, `已选 ${s.size} 本`, entries());
    openMenu(e, v.title, mediaEntries('book', v.id, v.title, [{ label: '打开', run: () => open(v.id) }]));
  }
  function volBatch(ids: number[], done: (ok: boolean) => void): MenuEntry[] {
    return [
      'sep',
      { label: '从媒体库移除…', run: async () => done(await removeMany('book', ids)) },
      { label: '删除文件…', danger: true, run: async () => done(await deleteMany('book', ids)) },
    ];
  }
</script>

<svelte:window onkeydown={e => openKey == null && sel.keys(e, () => order)} />

<section class="lib on" class:multi={sel.on}>
  {#if openWork}
    <WorkDetail
      title={openWork.name}
      poster={openWork.items.find(b => b.cover)?.cover ?? null}
      kind="小说"
      unit="卷"
      vols={openWork.items.map(bookVol)}
      nextId={nextOf(openWork)?.id ?? null}
      onback={() => (openKey = null)}
      onopen={open}
      onmenu={volMenu}
      batch={volBatch}
      onheadmenu={e => openMenu(e, openWork.name, workEntries(openWork).slice(1))}
    />
  {:else}
  <header>
    <div>
      <h2>小说</h2>
      <p>{books.length ? (works.length < books.length ? `${works.length} 部 · 共 ${books.length} 本` : `共 ${books.length} 本`) : '本地 TXT / EPUB'}</p>
    </div>
    {#if books.length}
      <button class="mp-chip msel" class:on={sel.on} onclick={() => (sel.on ? sel.clear() : (sel.mode = true))}><Icon name="check" /> {sel.on ? '退出多选' : '多选'}</button>
    {/if}
  </header>

  <FollowStrip kind="novel" names={works.map(w => w.name)} />

  {#if sel.on}
    <BatchBar {sel} unit="部" total={order.length} onall={() => sel.selectAll(order)} entries={batch()} />
  {/if}

  {#if !loaded}
    <p class="empty">正在读取…</p>
  {:else if !books.length}
    <p class="empty">{inTauri ? '还没有小说。把 TXT 或 EPUB 文件拖进窗口，或者添加媒体库目录。' : '在桌面程序中查看小说书架。'}</p>
  {:else}
    <div class="mp-grid">
      {#each works as w (w.key)}
        {@const i = wid(w)}
        {@const cover = w.items.find(b => b.cover)?.cover ?? null}
        <div class="mp-card mp-book" class:picked={sel.has(i)} role="button" tabindex="0" onclick={e => clickW(e, w)} onmousedown={noTextSelect} onkeydown={e => e.key === 'Enter' && openW(w)} oncontextmenu={e => workMenu(e, w)}>
          <Cover {cover} name={w.name} tag="div" cls="mp-poster">
            <SelCheck card checked={sel.has(i)} onclick={e => (e.shiftKey ? sel.range(order, i, true) : sel.toggle(i))} />
            {#if w.items.length > 1}<span class="mp-badge">{w.items.length} 卷</span>{/if}
            {#if !cover}<span class="mp-pt">{w.name}</span>{/if}
            <i class="mp-bar" style="width:{workPct(w)}%"></i>
            <MoreButton onopen={e => workMenu(e, w)} />
          </Cover>
          <div class="mp-card-t">{w.name}</div>
          <div class="mp-card-s">{workLine(w)}</div>
        </div>
      {/each}
    </div>
  {/if}
  {/if}
</section>

<style>
  .msel { display: inline-flex; align-items: center; gap: 5px; }
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
