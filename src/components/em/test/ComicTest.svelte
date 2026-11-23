<script lang="ts">
  // 规则管理页“测试”面板（漫画排版）：选一本漫画，看排版后的页面
  import { comicPage } from '../../../lib/api';
  import { emTest, type EmRule } from '../../../lib/em.svelte';
  import type { ComicTestBook, ComicTestResult } from '../../../lib/emcomic';
  let { rule }: { rule: EmRule } = $props();

  let books = $state.raw<ComicTestBook[]>([]);
  let all = $state(false);
  let pick = $state<number | null>(null);
  let res = $state.raw<ComicTestResult | null>(null);
  let busy = $state(false);
  let err = $state('');
  let limit = $state(60);

  $effect(() => {
    const id = rule.id;
    void (async () => {
      try {
        const r = await emTest<{ comics: ComicTestBook[] }>(id, {});
        books = r.comics;
        const m = books.filter(b => b.matched);
        all = !m.length;
        pick = (m[0] ?? books[0])?.id ?? null;
      } catch (e) {
        err = String(e);
      }
    })();
  });

  const shown = $derived(all ? books : books.filter(b => b.matched));

  async function run() {
    if (pick == null) return;
    busy = true;
    err = '';
    res = null;
    limit = 60;
    try {
      res = await emTest<ComicTestResult>(rule.id, { comic_id: pick });
    } catch (e) {
      err = String(e);
    }
    busy = false;
  }

  const label = (p: ComicTestResult['pages'][number]) =>
    [p.half === 'right' ? '拆·右' : p.half === 'left' ? '拆·左' : '', p.trim ? '裁' : ''].filter(Boolean).join(' ');
</script>

<div class="ct">
  {#if !books.length && !err}
    <p class="note">媒体库里还没有能测试的漫画（CBZ / ZIP / 图片文件夹）。</p>
  {:else}
    <div class="ct-bar">
      <select class="mp-q" bind:value={pick} onchange={() => (res = null)}>
        {#each shown as b (b.id)}
          <option value={b.id}>{b.series && b.series !== b.title ? `${b.series} · ` : ''}{b.title}{b.matched ? '' : '（不匹配）'}</option>
        {/each}
      </select>
      <label class="ct-all"><input type="checkbox" bind:checked={all} /> 显示全部漫画</label>
      <button class="mp-btn" disabled={busy || pick == null} onclick={run}>{busy ? '正在排版…' : '测试'}</button>
    </div>
    {#if !books.some(b => b.matched)}
      <p class="note">没有漫画匹配这个规则的 match，可以先拿任意一本看看效果。</p>
    {/if}
  {/if}
  {#if err}<p class="ct-err">{err}</p>{/if}

  {#if res}
    <p class="ct-sum">
      {#if res.marker}
        这本是按这个规则导出的，页面不再处理。
      {:else if !res.processed}
        这个规则只管阅读排版和导出，阅读时页面不变。
      {:else}
        原来 <b>{res.before}</b> 页 → 排版后 <b>{res.after}</b> 页：拆开 {res.split} 个跨页、裁边 {res.trimmed} 页、跳过 {res.skipped} 页
      {/if}
      {#if !res.matched}<span class="ct-warn">（这本不匹配规则的 match，平时不会套用）</span>{/if}
    </p>
    {#if res.skipped_pages.length}
      <p class="note">跳过：{res.skipped_pages.map(s => `${s.name}（${s.why}）`).join('、')}</p>
    {/if}
    <div class="ct-grid">
      {#each res.pages.slice(0, limit) as p (p.i)}
        <figure>
          <img src={comicPage(pick ?? 0, p.i, { id: rule.id, ver: res.ver })} alt="第 {p.i + 1} 页" loading="lazy" decoding="async" />
          <figcaption>
            <span>{p.i + 1}</span>
            {#if label(p)}<em>{label(p)}</em>{/if}
          </figcaption>
        </figure>
      {/each}
    </div>
    {#if res.pages.length > limit}
      <button class="mp-btn ghost ct-more" onclick={() => (limit += 120)}>再显示 {Math.min(120, res.pages.length - limit)} 页</button>
    {/if}
  {/if}
</div>

<style>
  .ct-bar {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .ct-bar select {
    flex: 1;
    min-width: 200px;
  }
  .ct-all {
    font-size: 12.5px;
    display: flex;
    gap: 5px;
    align-items: center;
    color: var(--mp-sub);
  }
  .note {
    color: var(--mp-sub);
    font-size: 12.5px;
    line-height: 1.6;
    margin: 8px 0;
  }
  .ct-err {
    color: #ff8a8a;
    font-size: 12.5px;
    white-space: pre-line;
  }
  .ct-sum {
    font-size: 13px;
    margin: 12px 0 6px;
  }
  .ct-warn {
    color: #ffc46b;
  }
  .ct-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(92px, 1fr));
    gap: 8px;
    max-height: 52vh;
    overflow: auto;
    padding: 2px;
  }
  figure {
    margin: 0;
    background: rgba(255, 255, 255, 0.04);
    border-radius: 8px;
    overflow: hidden;
  }
  figure img {
    width: 100%;
    aspect-ratio: 0.7;
    object-fit: contain;
    display: block;
    background: #111;
  }
  figcaption {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    padding: 3px 6px;
    color: var(--mp-sub);
  }
  figcaption em {
    font-style: normal;
    color: #7be0a3;
  }
  .ct-more {
    margin-top: 8px;
  }
</style>
