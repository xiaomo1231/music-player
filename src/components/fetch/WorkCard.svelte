<script lang="ts">
  // 搜索结果里的一张作品卡：一个来源的一部作品（漫画 / 小说 / 番剧 / 专辑）。
  // 收起时只显示封面、作品名、来源、话数和最新话；点开才去取章节，可以按区间、只要未下载勾选，分批显示
  import { untrack } from 'svelte';
  import { toast } from '../../lib/ui.svelte';
  import { fetchApi, fmtBytes, fmtDur, providerItem, providerKey, withQuality, EP_UNIT, type Hit, type SubRow } from '../../lib/fetch';
  import { applyChoice, choiceLabel, choiceMediaKind, isBiliHit, loadChoice, type BiliChoice } from '../../lib/bilidl';
  import BiliDlMenu from './BiliDlMenu.svelte';
  import { dedupe, knownEpisodes, latestEpisode, needsEpisodes, rangeUrls, sortEpisodes, PAGE, type WorkCard } from '../../lib/fetchworks';

  let {
    card,
    hits,
    mk,
    downloaded,
    sub,
    picked = '',
    open = false,
    ontoggle,
    ondownload,
    onsub,
  }: {
    card: WorkCard;
    /** 整次搜索的结果（card.indices 指向这里） */
    hits: Hit[];
    mk: string;
    /** 已经在下载列表里的条目（providerKey） */
    downloaded: Set<string>;
    sub?: SubRow;
    /** AI 推荐理由（有就标“推荐”） */
    picked?: string;
    open?: boolean;
    ontoggle: () => void;
    /** mediaKind：存到哪个库（不传按当前分类） */
    ondownload: (list: Hit[], mediaKind?: string) => Promise<void> | void;
    /** 追更 / 取消追更：带上作品条目地址和封面 */
    onsub: (workUrl: string, cover: string) => void;
  } = $props();

  const unit = $derived(mk === 'music' ? '首' : (EP_UNIT[mk] ?? '个'));
  const base = $derived(card.indices.map(i => hits[i]));
  const first = $derived(providerItem(base[0]));
  const cover = $derived(base.map(h => providerItem(h)?.cover).find(Boolean) ?? '');
  const author = $derived(first?.artist ?? '');
  const lazy = $derived(needsEpisodes(hits, card));

  // B 站：下载前选下载什么（音视频 / 仅音频 / 仅视频 / 仅元数据）、清晰度、字幕、弹幕、封面
  const bili = $derived(isBiliHit(base[0]));
  let choice = $state<BiliChoice>(untrack(() => loadChoice(mk)));
  /** 打开的选项弹层：单条下载 / 全部下载 / 只改选项（勾选后“下载选中”用） */
  let menu = $state<'' | 'single' | 'all' | 'sel'>('');
  const biliDl = (list: Hit[], c: BiliChoice) => ondownload(list.map(h => applyChoice(h, c)), choiceMediaKind(c, mk));

  // 点开后取到的章节
  let fetched = $state<Hit[] | null>(null);
  let loading = $state(false);
  let error = $state('');
  const eps = $derived(fetched ?? knownEpisodes(hits, card));
  const loaded = $derived(!lazy || fetched !== null);
  const latest = $derived(latestEpisode(eps));
  /** 只有一条、又没有集数：整本 / 单曲，直接下载，不用展开 */
  const single = $derived(!lazy && base.length === 1 && base[0].episode == null);

  async function loadEps(): Promise<Hit[] | null> {
    if (loaded) return eps;
    if (loading) return null;
    loading = true;
    error = '';
    try {
      const parts = await Promise.all(base.map(h => (h.expand ? fetchApi.episodes(h.source_id, h.url) : Promise.resolve([h]))));
      fetched = dedupe(parts.flat());
      return fetched;
    } catch (err) {
      error = String(err);
      return null;
    } finally {
      loading = false;
    }
  }

  // 点开时取一次；失败了等用户点“重试”
  $effect(() => {
    if (open && !loaded && !error && !loading) untrack(() => void loadEps());
  });

  // ---------- 勾选、排序、分批显示 ----------
  let sel = $state<Record<string, boolean>>({});
  let desc = $state(false);
  let shown = $state(PAGE);
  let from = $state('');
  let to = $state('');
  let qkey = $state('');

  const isDone = (h: Hit) => {
    const k = providerKey(h.url);
    return !!k && downloaded.has(k);
  };
  const list = $derived(sortEpisodes(eps, desc));
  const selCount = $derived(eps.filter(h => sel[h.url]).length);
  const doneCount = $derived(eps.filter(isDone).length);
  const qualities = $derived(providerItem(eps[0] ?? base[0])?.qualities ?? []);

  function setAll(urls: string[], on = true) {
    const next = { ...sel };
    for (const u of urls) next[u] = on;
    sel = next;
  }
  const selectAll = () => setAll(eps.map(h => h.url));
  const clear = () => (sel = {});
  function selectNew() {
    sel = {};
    setAll(eps.filter(h => !isDone(h)).map(h => h.url));
    if (!eps.some(h => !isDone(h))) toast(`这部作品的${unit}都已经在下载列表里了`);
  }
  function selectRange() {
    const a = Number(from);
    const b = Number(to || from);
    if (!from.trim() || !Number.isFinite(a) || !Number.isFinite(b)) return toast('先填要选的范围，例如 1 到 50');
    const urls = rangeUrls(eps, a, b);
    if (!urls.length) return toast('这个范围里没有章节');
    setAll(urls);
    // 选中的那段要能看到
    const lastIdx = Math.max(...urls.map(u => list.findIndex(h => h.url === u)));
    if (lastIdx >= shown) shown = Math.ceil((lastIdx + 1) / PAGE) * PAGE;
  }

  const withQ = (h: Hit) => (qkey ? withQuality(h, qkey) : h);

  async function downloadSel() {
    const pick = sortEpisodes(eps).filter(h => sel[h.url]);
    if (!pick.length) return toast(`先勾选要下载的${unit}`);
    if (bili) await biliDl(pick, choice);
    else await ondownload(pick.map(withQ));
    sel = {};
  }

  async function downloadAll() {
    const all = await loadEps();
    if (!all) return toast(error || '正在读取章节，稍后再点');
    if (bili) await biliDl(sortEpisodes(all), choice);
    else await ondownload(sortEpisodes(all).map(withQ));
  }

  async function onMenuConfirm(c: BiliChoice) {
    const which = menu;
    menu = '';
    choice = c;
    if (which === 'single') await biliDl([base[0]], c);
    else if (which === 'all') await downloadAll();
  }

  async function toggleSub() {
    // 追更认作品条目：没展开过的作品条目直接用；已经是各集的用不上
    const work = base.find(h => h.expand);
    onsub(work?.url ?? '', cover);
  }

  function label(h: Hit): string {
    return providerItem(h)?.title || h.title;
  }
  function epTag(h: Hit): string {
    return h.episode != null ? `${h.episode}` : '';
  }

  const summary = $derived.by(() => {
    const parts = [card.sourceName];
    if (author && mk !== 'anime') parts.push(author);
    if (single && bili) {
      const d = fmtDur(first?.duration);
      if (d) parts.push(d);
    } else if (single) {
      const q = providerItem(base[0])?.qualities[0];
      if (q?.label) parts.push(q.label);
      if (base[0].size) parts.push(fmtBytes(base[0].size));
    } else if (loaded) {
      parts.push(`共 ${eps.length} ${unit}`);
      if (latest != null) parts.push(`最新 第 ${latest} ${unit}`);
      if (doneCount) parts.push(`已下 ${doneCount}`);
    } else {
      parts.push(loading ? '正在读取章节…' : '点开查看章节');
    }
    return parts.join(' · ');
  });
</script>

<div class="wc" class:open class:subbed={!!sub}>
  <div class="wc-head">
    {#if cover}
      <img class="wc-cv" src={cover} alt="" loading="lazy" referrerpolicy="no-referrer" />
    {:else}
      <div class="wc-cv ph">{card.work.slice(0, 2)}</div>
    {/if}
    <div class="wc-body">
      <div class="wc-title">
        <b title={card.work}>{card.work}</b>
        {#if picked}<span class="badge2 ai" title={picked}>推荐</span>{/if}
        {#if sub}<span class="badge2 ok">已追更</span>{/if}
      </div>
      <small class="wc-sub">{summary}</small>
      <div class="wc-acts">
        {#if single && bili}
          <button class="mp-btn ghost sm" data-bili-dl class:on={menu === 'single'} onclick={() => (menu = menu === 'single' ? '' : 'single')} title="选择下载音视频、仅音频、仅视频或仅元数据">
            {isDone(base[0]) ? '再下载一次' : '下载'} ▾
          </button>
        {:else if single}
          <button class="mp-btn ghost sm" onclick={() => ondownload([base[0]])}>{isDone(base[0]) ? '再下载一次' : '下载'}</button>
        {:else}
          <button class="mp-btn ghost sm" class:on={open} onclick={ontoggle}>{open ? '收起' : `选${unit}下载`}</button>
          {#if bili}
            <button class="mp-btn ghost sm" data-bili-dl class:on={menu === 'all'} onclick={() => (menu = menu === 'all' ? '' : 'all')} disabled={loading}>全部下载 ▾</button>
          {:else}
            <button class="mp-btn ghost sm" onclick={downloadAll} disabled={loading}>全部下载</button>
          {/if}
          {#if mk !== 'music'}
            <button class="mp-btn ghost sm" onclick={toggleSub}>{sub ? '取消追更' : '追更'}</button>
          {/if}
        {/if}
        {#if menu === 'single' || menu === 'all'}
          <BiliDlMenu {mk} bind:choice confirmText={menu === 'all' ? '全部下载' : '开始下载'} onconfirm={onMenuConfirm} onclose={() => (menu = '')} />
        {/if}
      </div>
    </div>
  </div>

  {#if open && !single}
    <div class="wc-panel">
      {#if loading}
        <div class="empty sm">正在读取章节…</div>
      {:else if error}
        <div class="note">读取章节失败：{error} <button class="mp-btn ghost sm" onclick={loadEps}>重试</button></div>
      {:else if !eps.length}
        <div class="empty sm">这部作品没有列出章节</div>
      {:else}
        <div class="wc-tools">
          <button class="mp-chip" onclick={selectAll}>全选</button>
          <button class="mp-chip" onclick={selectNew} title="跳过已经在下载列表里的">只选未下载</button>
          <button class="mp-chip" onclick={clear} disabled={!selCount}>清空</button>
          <span class="rng">
            第 <input class="tin" inputmode="decimal" aria-label="起始" placeholder="1" bind:value={from} onkeydown={e => e.key === 'Enter' && selectRange()} />
            – <input class="tin" inputmode="decimal" aria-label="结束" placeholder={latest != null ? String(latest) : String(eps.length)} bind:value={to} onkeydown={e => e.key === 'Enter' && selectRange()} />
            {unit} <button class="mp-chip" onclick={selectRange}>选中</button>
          </span>
          <button class="mp-chip" onclick={() => (desc = !desc)} title="排序">{desc ? '倒序' : '正序'}</button>
          {#if bili}
            <span class="bili-opt">
              <button class="mp-chip" data-bili-dl class:on={menu === 'sel'} onclick={() => (menu = menu === 'sel' ? '' : 'sel')} title="下载选中时用的内容和清晰度">
                {choiceLabel(choice)} ▾
              </button>
              {#if menu === 'sel'}
                <BiliDlMenu {mk} bind:choice confirmText="确定" onconfirm={onMenuConfirm} onclose={() => (menu = '')} />
              {/if}
            </span>
          {:else if qualities.length > 1}
            <select class="mp-chip" aria-label="版本" bind:value={qkey}>
              <option value="">默认版本</option>
              {#each qualities as x (x.key)}<option value={x.key}>{x.label || x.format.toUpperCase()}</option>{/each}
            </select>
          {/if}
          <span class="grow"></span>
          <button class="mp-btn sm" onclick={downloadSel} disabled={!selCount}>下载选中（{selCount}）</button>
        </div>
        <div class="eps-grid">
          {#each list.slice(0, shown) as h (h.url)}
            {@const done = isDone(h)}
            <label class="ep" class:on={sel[h.url]} class:done title={label(h)}>
              <input type="checkbox" checked={!!sel[h.url]} onchange={e => (sel[h.url] = e.currentTarget.checked)} />
              {#if epTag(h)}<span class="no">{epTag(h)}</span>{/if}
              <span class="t">{label(h)}</span>
              {#if done}<span class="got">已下</span>{/if}
            </label>
          {/each}
        </div>
        {#if list.length > shown}
          <div class="more">
            <button class="mp-chip" onclick={() => (shown += PAGE)}>再显示 {Math.min(PAGE, list.length - shown)} {unit}</button>
            <button class="mp-chip" onclick={() => (shown = list.length)}>全部显示（还有 {list.length - shown} {unit}）</button>
          </div>
        {/if}
      {/if}
    </div>
  {/if}
</div>

<style>
  .wc {
    min-width: 0;
    padding: 10px;
    border-radius: 12px;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.06);
  }
  .wc.open {
    grid-column: 1 / -1;
    background: rgba(255, 255, 255, 0.07);
  }
  .wc-head {
    display: flex;
    gap: 10px;
    min-width: 0;
  }
  .wc-cv {
    width: 54px;
    height: 74px;
    border-radius: 8px;
    object-fit: cover;
    flex: none;
  }
  .wc-cv.ph {
    display: grid;
    place-items: center;
    background: rgba(255, 255, 255, 0.08);
    color: var(--mp-sub);
    font-size: 13px;
  }
  .wc-body {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    flex: 1;
  }
  .wc-title {
    display: flex;
    gap: 6px;
    align-items: center;
    min-width: 0;
  }
  .wc-title b {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13.5px;
  }
  .wc-sub {
    color: var(--mp-sub);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wc-acts {
    position: relative;
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: auto;
  }
  .sm {
    padding: 4px 10px;
    font-size: 12px;
    white-space: nowrap;
  }
  .mp-btn.on {
    border-color: var(--mp-accent);
  }
  .wc-panel {
    margin-top: 10px;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    padding-top: 10px;
  }
  .wc-tools {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    align-items: center;
    margin-bottom: 8px;
    font-size: 12.5px;
  }
  .rng {
    display: inline-flex;
    gap: 4px;
    align-items: center;
    color: var(--mp-sub);
  }
  .rng .tin {
    width: 56px;
    padding: 3px 6px;
    font-size: 12px;
  }
  .grow {
    flex: 1;
  }
  .bili-opt {
    position: relative;
  }
  select.mp-chip {
    color: inherit;
    font: inherit;
    font-size: 12.5px;
  }
  select.mp-chip option {
    color: #000;
  }
  .eps-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 4px 6px;
  }
  .ep {
    display: flex;
    gap: 6px;
    align-items: center;
    min-width: 0;
    padding: 5px 8px;
    border-radius: 8px;
    font-size: 12.5px;
    cursor: pointer;
    background: rgba(255, 255, 255, 0.03);
  }
  .ep:hover {
    background: rgba(255, 255, 255, 0.08);
  }
  .ep.on {
    background: color-mix(in srgb, var(--mp-accent) 22%, transparent);
  }
  .ep.done .t {
    opacity: 0.55;
  }
  .ep input {
    margin: 0;
    flex: none;
  }
  .ep .no {
    flex: none;
    min-width: 22px;
    color: var(--mp-sub);
    font-variant-numeric: tabular-nums;
    font-size: 11.5px;
  }
  .ep .t {
    min-width: 0;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ep .got {
    flex: none;
    font-size: 10.5px;
    color: var(--mp-sub);
  }
  .more {
    display: flex;
    gap: 6px;
    justify-content: center;
    margin-top: 8px;
  }
  .empty.sm {
    padding: 12px 0;
  }
</style>
