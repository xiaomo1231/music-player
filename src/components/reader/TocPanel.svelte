<script lang="ts">
  import type { Chapter } from '../../lib/api';
  import type { Bookmark } from '../../lib/reader';

  let {
    chapters,
    current,
    bookmarks,
    focusSearch = 0,
    onchapter,
    onpara,
    onbookmark,
    onremove,
  }: {
    chapters: Chapter[];
    current: number;
    bookmarks: Bookmark[];
    /** 每次变化都会把焦点放到搜索框（Ctrl+F） */
    focusSearch?: number;
    onchapter: (ci: number) => void;
    onpara: (ci: number, pi: number) => void;
    onbookmark: (b: Bookmark) => void;
    onremove: (i: number) => void;
  } = $props();

  let q = $state('');
  let tab = $state<'toc' | 'search' | 'mark'>('toc');
  let searchEl: HTMLInputElement | undefined = $state();

  $effect(() => {
    if (focusSearch > 0) {
      tab = 'search';
      queueMicrotask(() => searchEl?.focus());
    }
  });

  type Hit = { ci: number; pi: number; snip: string };
  const results = $derived.by<Hit[]>(() => {
    const k = q.trim().toLowerCase();
    if (!k) return [];
    const out: Hit[] = [];
    chapters.forEach((ch, ci) => {
      ch.paras.forEach((p, pi) => {
        const at = p.toLowerCase().indexOf(k);
        if (at >= 0 && out.length < 200) {
          const from = Math.max(0, at - 12);
          out.push({ ci, pi, snip: (from > 0 ? '…' : '') + p.slice(from, at + k.length + 24) });
        }
      });
    });
    return out;
  });
</script>

<div class="toc-inner">
  <div class="tabs">
    <button type="button" class:on={tab === 'toc'} onclick={() => (tab = 'toc')}>目录</button>
    <button type="button" class:on={tab === 'search'} onclick={() => (tab = 'search')}>搜索</button>
    <button type="button" class:on={tab === 'mark'} onclick={() => (tab = 'mark')}>书签 {bookmarks.length}</button>
  </div>

  {#if tab === 'toc'}
    <ol class="list">
      {#each chapters as ch, ci (ci)}
        <li>
          <button type="button" class:cur={ci === current} onclick={() => onchapter(ci)}>{ch.title || `第 ${ci + 1} 节`}</button>
        </li>
      {/each}
    </ol>
  {:else if tab === 'search'}
    <input class="search" type="search" placeholder="全文搜索" bind:value={q} bind:this={searchEl} />
    {#if q.trim() && !results.length}
      <p class="hint">没有找到“{q.trim()}”。</p>
    {/if}
    {#if results.length >= 200}
      <p class="hint">只显示前 200 处。</p>
    {/if}
    <ol class="list">
      {#each results as r, i (i)}
        <li>
          <button type="button" onclick={() => onpara(r.ci, r.pi)}>
            <small>{chapters[r.ci].title}</small>
            {r.snip}
          </button>
        </li>
      {/each}
    </ol>
  {:else}
    {#if !bookmarks.length}
      <p class="hint">按 B 键可以在当前位置加书签。</p>
    {/if}
    <ol class="list">
      {#each bookmarks as b, i (i)}
        <li class="bm">
          <button type="button" onclick={() => onbookmark(b)}>
            <small>{chapters[b.chapter]?.title ?? ''}</small>
            {b.label || '书签'}
          </button>
          <button type="button" class="del" aria-label="删除书签" onclick={() => onremove(i)}>删除</button>
        </li>
      {/each}
    </ol>
  {/if}
</div>

<style>
  .toc-inner {
    padding: 14px 12px 30px;
    height: 100%;
    overflow: auto;
    box-sizing: border-box;
    font-size: 13px;
  }
  .tabs {
    display: flex;
    gap: 4px;
    margin-bottom: 10px;
  }
  .tabs button,
  .list button,
  .del {
    border: 1px solid currentColor;
    background: none;
    color: inherit;
    border-radius: 8px;
    padding: 4px 9px;
    font-size: 12.5px;
    opacity: 0.8;
    cursor: pointer;
  }
  .tabs button.on,
  .list button.cur {
    opacity: 1;
    font-weight: 600;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 6px;
  }
  .list button {
    width: 100%;
    text-align: left;
    border-color: transparent;
    line-height: 1.5;
    word-break: break-all;
  }
  .list small {
    display: block;
    opacity: 0.6;
    font-size: 11px;
  }
  .bm {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 6px;
    align-items: start;
  }
  .search {
    width: 100%;
    box-sizing: border-box;
    padding: 6px 9px;
    border-radius: 8px;
    border: 1px solid currentColor;
    background: transparent;
    color: inherit;
    font: inherit;
    margin-bottom: 8px;
  }
  .hint {
    opacity: 0.65;
    font-size: 12px;
  }
</style>
