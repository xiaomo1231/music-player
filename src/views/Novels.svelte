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

  let books = $state<BookItem[]>([]);
  let loaded = $state(false);

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
      const list = await invoke<BookItem[]>('list_books');
      // 最近阅读排在前面；没读过的按书名
      books = [...list].sort((a, b) => (b.last_read ?? 0) - (a.last_read ?? 0) || a.title.localeCompare(b.title, 'zh-CN'));
    } catch {
      books = [];
    }
    loaded = true;
  }

  /** 阅读进度：章节 + 章内位置 */
  function progress(b: BookItem): { label: string; pct: number; done: boolean } {
    if (b.last_read == null || b.chapters <= 0) return { label: '未读', pct: 0, done: false };
    const pct = Math.min(100, Math.round(((b.chapter + b.offset) / b.chapters) * 100));
    const done = b.chapter >= b.chapters - 1 && b.offset >= 0.98;
    return done ? { label: '已读完', pct: 100, done: true } : { label: `已读 ${pct}%`, pct, done: false };
  }

  function open(id: number) {
    ui.bookId = id;
  }
  // ---------- 多选 ----------
  const sel = new Selection();
  const order = $derived(books.map(x => x.id));
  $effect(() => { const o = order; untrack(() => sel.prune(o)); });
  function clickItem(e: MouseEvent, id: number) {
    if (!sel.click(e, id, order)) open(id);
  }
  function batch(): MenuEntry[] {
    const ids = sel.ids(order);
    const done = (ok: boolean) => ok && sel.clear();
    return [
      { label: '从媒体库移除…', run: async () => done(await removeMany('book', ids)) },
      { label: '删除文件…', danger: true, run: async () => done(await deleteMany('book', ids)) },
    ];
  }

  function bookMenu(e: MouseEvent, b: BookItem) {
    if (sel.has(b.id) && sel.size > 1) return openMenu(e, `已选 ${sel.size} 本`, batch());
    openMenu(e, b.title, mediaEntries('book', b.id, b.title, [{ label: '打开', run: () => open(b.id) }]));
  }
</script>

<svelte:window onkeydown={e => sel.keys(e, () => order)} />

<section class="lib on" class:multi={sel.on}>
  <header>
    <div>
      <h2>小说</h2>
      <p>{books.length ? `共 ${books.length} 本` : '本地 TXT / EPUB'}</p>
    </div>
    {#if books.length}
      <button class="mp-chip msel" class:on={sel.on} onclick={() => (sel.on ? sel.clear() : (sel.mode = true))}><Icon name="check" /> {sel.on ? '退出多选' : '多选'}</button>
    {/if}
  </header>

  {#if sel.on}
    <BatchBar {sel} unit="本" total={order.length} onall={() => sel.selectAll(order)} entries={batch()} />
  {/if}

  {#if !loaded}
    <p class="empty">正在读取…</p>
  {:else if !books.length}
    <p class="empty">{inTauri ? '还没有小说。把 TXT 或 EPUB 文件拖进窗口，或者添加媒体库目录。' : '在桌面程序中查看小说书架。'}</p>
  {:else}
    <div class="mp-grid">
      {#each books as b (b.id)}
        {@const pr = progress(b)}
        <div class="mp-card mp-book" class:picked={sel.has(b.id)} role="button" tabindex="0" onclick={e => clickItem(e, b.id)} onmousedown={noTextSelect} onkeydown={e => e.key === 'Enter' && open(b.id)} oncontextmenu={e => bookMenu(e, b)}>
          <Cover cover={b.cover} name={b.title} tag="div" cls="mp-poster">
            <SelCheck card checked={sel.has(b.id)} onclick={e => (e.shiftKey ? sel.range(order, b.id, true) : sel.toggle(b.id))} />
            {#if !b.cover}<span class="mp-pt">{b.title}</span>{/if}
            <i class="mp-bar" style="width:{pr.pct}%"></i>
            <MoreButton onopen={e => bookMenu(e, b)} />
          </Cover>
          <div class="mp-card-t">{b.title}</div>
          <div class="mp-card-s">{b.author || '佚名'} · {pr.label}</div>
        </div>
      {/each}
    </div>
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
