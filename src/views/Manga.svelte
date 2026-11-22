<script lang="ts">
  import MoreButton from '../components/MoreButton.svelte';
  import { openMenu, mediaEntries } from '../lib/items.svelte';
  import { invoke, inTauri, type ComicItem } from '../lib/api';
  import { lib } from '../lib/library.svelte';
  import { ui } from '../lib/ui.svelte';
  import Cover from '../components/Cover.svelte';

  let comics = $state<ComicItem[]>([]);
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
      comics = await invoke<ComicItem[]>('list_comics');
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
  function comicMenu(e: MouseEvent, c: ComicItem) {
    openMenu(e, c.title, mediaEntries('comic', c.id, c.title, [{ label: '打开', run: () => open(c.id) }]));
  }
</script>

<section class="lib on">
  <header>
    <div>
      <h2>漫画</h2>
      <p>{comics.length ? `${groups.length} 个系列 · 共 ${comics.length} 卷` : '本地 CBZ / ZIP / PDF / 图片文件夹'}</p>
    </div>
  </header>

  {#if !loaded}
    <p class="empty">正在读取…</p>
  {:else if !comics.length}
    <p class="empty">{inTauri ? '还没有漫画。把 CBZ、ZIP、PDF 文件或图片文件夹拖进窗口，或者添加媒体库目录。' : '在桌面程序中查看漫画书架。'}</p>
  {:else}
    {#each groups as g (g.name)}
      <h3 class="series">{g.name} <small>{g.items.length} 卷</small></h3>
      <div class="mp-grid">
        {#each g.items as c (c.id)}
          <div class="mp-card mp-book" role="button" tabindex="0" onclick={() => open(c.id)} onkeydown={e => e.key === 'Enter' && open(c.id)} oncontextmenu={e => comicMenu(e, c)}>
            <Cover cover={c.cover} name={c.title} tag="div" cls="mp-poster">
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

<style>
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
