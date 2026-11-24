<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import Icon from '../../lib/Icon.svelte';
  import Cover from '../Cover.svelte';
  import MoreButton from '../MoreButton.svelte';
  import BatchBar from '../BatchBar.svelte';
  import SelCheck from '../SelCheck.svelte';
  import { Selection, noTextSelect } from '../../lib/select.svelte';
  import { coverUrl } from '../../lib/api';
  import { coverCSS, hueOf } from '../../lib/util';
  import type { MenuEntry } from '../../lib/items.svelte';
  import { loadShelfView, saveShelfView, type Vol } from '../../lib/shelf';

  // 漫画 / 小说共用的作品详情页：横幅、继续阅读、正篇 / 番外分组、网格 / 列表、多选
  let {
    title,
    poster,
    kind,
    unit,
    vols,
    nextId,
    onback,
    onopen,
    onmenu,
    batch,
    onmark,
    onheadmenu,
    onposter,
  }: {
    title: string;
    /** 作品海报（绝对路径或封面目录里的文件名） */
    poster: string | null;
    kind: '漫画' | '小说';
    unit: '话' | '卷';
    /** 已按话 / 卷号排好序 */
    vols: Vol[];
    /** 接着读哪一卷；null 表示全读完了 */
    nextId: number | null;
    onback: () => void;
    onopen: (id: number) => void;
    onmenu: (e: MouseEvent, v: Vol, sel: Selection, entries: () => MenuEntry[]) => void;
    /** 选中的这些卷可做的批量操作；done(true) 后退出多选 */
    batch: (ids: number[], done: (ok: boolean) => void) => MenuEntry[];
    /** 标记已读 / 未读（小说没有） */
    onmark?: (ids: number[], read: boolean) => void;
    onheadmenu?: (e: MouseEvent) => void;
    onposter?: () => void;
  } = $props();

  let root = $state<HTMLElement>();
  let tab = $state<'main' | 'extra'>('main');
  let desc = $state(false);
  let view = $state<'grid' | 'list'>(loadShelfView());
  $effect(() => saveShelfView(view));

  const mains = $derived(vols.filter(v => !v.extra));
  const extras = $derived(vols.filter(v => v.extra));
  // 全是“番外”的作品就不拆了
  const split = $derived(extras.length > 0 && mains.length > 0);
  const cur = $derived.by(() => {
    const list = !split ? vols : tab === 'extra' ? extras : mains;
    return desc ? [...list].reverse() : list;
  });
  const readN = $derived(vols.filter(v => v.done).length);
  const started = $derived(vols.some(v => v.started));
  const next = $derived(nextId == null ? null : (vols.find(v => v.id === nextId) ?? null));
  const resumeText = $derived(next == null ? `从第一${unit}重新读` : `${started ? '继续阅读' : '开始阅读'} ${next.title}`);
  const bg = $derived(poster ? `url("${coverUrl(poster)}")` : coverCSS(hueOf(title)));

  onMount(() => root?.closest('.lib')?.scrollTo(0, 0));

  // 继续阅读的那一卷在番外里时，默认切到番外
  $effect(() => {
    const n = next;
    untrack(() => {
      if (n && split) tab = n.extra ? 'extra' : 'main';
    });
  });

  function resume() {
    const v = next ?? vols[0];
    if (v) onopen(v.id);
  }

  // ---------- 多选 ----------
  const sel = new Selection();
  const order = $derived(cur.map(v => v.id));
  $effect(() => { void tab; untrack(() => sel.clear()); });
  $effect(() => { const o = order; untrack(() => sel.prune(o)); });
  const done = (ok: boolean) => ok && sel.clear();
  function entries(): MenuEntry[] {
    const ids = sel.ids(order);
    const vs = cur.filter(v => sel.has(v.id));
    const mark: MenuEntry[] = onmark
      ? [
          ...(vs.some(v => !v.done) ? [{ label: '标记为已读', run: () => { onmark(ids, true); sel.clear(); } }] : []),
          ...(vs.some(v => v.started) ? [{ label: '标记为未读', run: () => { onmark(ids, false); sel.clear(); } }] : []),
        ]
      : [];
    return [{ label: '打开', run: () => onopen(ids[0]) }, ...mark, ...batch(ids, done)];
  }
  function click(e: MouseEvent, v: Vol) {
    if (!sel.click(e, v.id, order)) onopen(v.id);
  }
  function menu(e: MouseEvent, v: Vol) {
    onmenu(e, v, sel, entries);
  }
</script>

<svelte:window onkeydown={e => sel.keys(e, () => order)} />

<div class="work" class:multi={sel.on} bind:this={root}>
  <button class="mp-chip" onclick={onback}>← 返回{kind}书架</button>

  <div class="banner wbanner" style:margin-top="12px" style:background-image={bg}>
    {#if onposter}
      <button class="pst pbtn" style:background-image={bg} title="更换海报" aria-label="更换海报" onclick={onposter}></button>
    {:else}
      <div class="pst" style:background-image={bg}></div>
    {/if}
    <div>
      <h2>{title}</h2>
      <div class="acts" style="margin-bottom:10px">
        <span class="badge2">{kind}</span>
        <span class="badge2">共 {vols.length} {unit}{split ? `（番外 ${extras.length}）` : ''}</span>
        <span class="badge2">已读 {readN} {unit}</span>
        {#if vols.length > readN}<span class="badge2 no">{vols.length - readN} {unit}未读</span>{:else}<span class="badge2 ok">已读完</span>{/if}
      </div>
      <div class="acts">
        <button class="mp-btn" onclick={resume} disabled={!vols.length}><Icon name="play" /> {resumeText}</button>
        {#if onmark}
          {#if vols.length > readN}
            <button class="mp-btn ghost" onclick={() => onmark(vols.map(v => v.id), true)}>全部标记已读</button>
          {:else}
            <button class="mp-btn ghost" onclick={() => onmark(vols.map(v => v.id), false)}>全部标记未读</button>
          {/if}
        {/if}
        {#if onheadmenu}<MoreButton floating={false} title="更多" onopen={onheadmenu} />{/if}
      </div>
    </div>
  </div>

  <div class="tabs">
    {#if split}
      <button class="mp-chip" class:on={tab === 'main'} onclick={() => (tab = 'main')}>正篇 {mains.length}</button>
      <button class="mp-chip" class:on={tab === 'extra'} onclick={() => (tab = 'extra')}>番外特典 {extras.length}</button>
    {/if}
    <span class="badge2">{cur.length} {unit} · 已读 {cur.filter(v => v.done).length}</span>
    <span class="grow"></span>
    <button class="mp-chip" onclick={() => (desc = !desc)} title="排序">{desc ? '倒序' : '正序'}</button>
    <span class="seg2">
      <button class="mp-chip" class:on={view === 'grid'} onclick={() => (view = 'grid')}>网格</button>
      <button class="mp-chip" class:on={view === 'list'} onclick={() => (view = 'list')}>列表</button>
    </span>
    {#if cur.length}
      <button class="mp-chip msel" class:on={sel.on} onclick={() => (sel.on ? sel.clear() : (sel.mode = true))}><Icon name="check" /> {sel.on ? '退出多选' : '多选'}</button>
    {/if}
  </div>

  {#if sel.on}
    <BatchBar {sel} {unit} total={order.length} onall={() => sel.selectAll(order)} entries={entries()} />
  {/if}

  {#if view === 'grid'}
    <div class="mp-grid">
      {#each cur as v (v.id)}
        <div class="mp-card mp-book" class:picked={sel.has(v.id)} class:cur={v.id === nextId} role="button" tabindex="0" onclick={e => click(e, v)} onmousedown={noTextSelect} onkeydown={e => e.key === 'Enter' && onopen(v.id)} oncontextmenu={e => menu(e, v)}>
          <Cover cover={v.cover ?? poster} name={title + v.id} tag="div" cls="mp-poster">
            <SelCheck card checked={sel.has(v.id)} onclick={e => (e.shiftKey ? sel.range(order, v.id, true) : sel.toggle(v.id))} />
            <span class="mp-badge">{v.badge}</span>
            {#if !v.cover && !poster}<span class="mp-pt">{v.title}</span>{/if}
            <i class="mp-bar" style:width="{v.pct}%"></i>
            <MoreButton onopen={e => menu(e, v)} />
          </Cover>
          <div class="mp-card-t">{v.title}</div>
          <div class="mp-card-s" class:hl={v.id === nextId}>{v.id === nextId && v.started ? `读到这里 · ${v.status}` : v.status}</div>
        </div>
      {/each}
    </div>
  {:else}
    <div class="vlist">
      {#each cur as v, i (v.id)}
        <div class="vrow" class:picked={sel.has(v.id)} class:done={v.done} class:cur={v.id === nextId} role="button" tabindex="0" onclick={e => click(e, v)} onmousedown={noTextSelect} onkeydown={e => e.key === 'Enter' && onopen(v.id)} oncontextmenu={e => menu(e, v)}>
          {#if sel.on}
            <SelCheck checked={sel.has(v.id)} onclick={e => (e.shiftKey ? sel.range(order, v.id, true) : sel.toggle(v.id))} />
          {:else}
            <span class="no">{desc ? cur.length - i : i + 1}</span>
          {/if}
          <span class="t">{v.title}</span>
          <span class="badge2">{v.badge}</span>
          <span class="vst">{v.status}</span>
          <span class="pb"><i style:width="{v.pct}%"></i></span>
          <MoreButton floating={false} onopen={e => menu(e, v)} />
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .msel { display: inline-flex; align-items: center; gap: 5px; }
  .grow { flex: 1; }
  .seg2 { display: inline-flex; gap: 4px; }
  .wbanner { background-size: cover; background-position: center 30%; }
  .pbtn { all: unset; position: relative; cursor: pointer; aspect-ratio: 2 / 3; border-radius: 12px; background-size: cover; background-position: center; box-shadow: 0 12px 30px #0008; }
  .mp-card { background: none; border: 0; padding: 0; color: inherit; font: inherit; text-align: left; }
  .mp-card-s.hl { color: var(--mp-accent); }
  .vlist { display: flex; flex-direction: column; gap: 2px; }
  .vrow { display: grid; grid-template-columns: 34px minmax(0, 1fr) auto 120px 80px 28px; align-items: center; gap: 12px; padding: 8px 10px; border-radius: 10px; cursor: pointer; }
  .vrow:hover { background: rgba(255, 255, 255, .06); }
  .vrow.picked { background: rgba(255, 255, 255, .1); }
  .vrow.cur .t { color: var(--mp-accent); }
  .vrow.done .t { opacity: .6; }
  .vrow .no { color: var(--mp-sub); font-size: 12px; text-align: right; font-variant-numeric: tabular-nums; }
  .vrow .t { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .vrow .vst { color: var(--mp-sub); font-size: 12px; text-align: right; }
  .vrow .pb { height: 3px; border-radius: 2px; background: rgba(255, 255, 255, .12); overflow: hidden; }
  .vrow .pb i { display: block; height: 100%; background: var(--mp-accent); }
</style>
