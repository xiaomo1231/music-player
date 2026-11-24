<script lang="ts">
  // 搜索并下载：同时搜所有来源，按作品分组；AI 挑推荐版本；整季 / 选中下载；追更订阅。
  // 番剧 / 漫画 / 小说 / 音乐 / 视频共用；有资料源时在结果上方显示作品资料卡
  import { onMount } from 'svelte';
  import { inTauri } from '../../lib/api';
  import TorrentQueue from './TorrentQueue.svelte';
  import { toast } from '../../lib/ui.svelte';
  import {
    fetchApi,
    fmtBytes,
    fmtDur,
    epText,
    hitKindText,
    isBtUrl,
    MEDIA_TEXT,
    metaApi,
    supportsKind,
    providerItem,
    providerQualityKey,
    withQuality,
    type DlItem,
    type Group,
    type Hit,
    type MetaCard,
    type MetaEpisode,
    type MetaInfo,
    type ProviderInfo,
    type ProviderItem,
    type SearchOut,
    type Source,
    type SourceStat,
    type SubRow,
  } from '../../lib/fetch';
  import { plugins, pickPlugin, CODE_TEXT } from '../../lib/plugin.svelte';
  import { em } from '../../lib/em.svelte';

  let { onqueued }: { onqueued?: () => void } = $props();

  let mk = $state('anime');
  let q = $state('');
  let out = $state<SearchOut | null>(null);
  let searching = $state(false);
  let picks = $state<Record<number, string>>({});
  let sel = $state<Record<number, boolean>>({});
  let prefs = $state('');
  let picking = $state(false);
  let subs = $state<SubRow[]>([]);
  // 下载源结果：曲目信息（从结果地址里解出）和每行选的档位
  let items = $derived<(ProviderItem | null)[]>(out ? out.hits.map(providerItem) : []);
  let qsel = $state<Record<number, string>>({});
  // 资料源：搜索时一起查作品资料
  let metas = $state<MetaInfo[]>([]);
  let providers = $state<ProviderInfo[]>([]);
  let cards = $state<MetaCard[]>([]);
  let cardIdx = $state(0);
  let eps = $state<Record<string, MetaEpisode[] | 'loading'>>({});
  const card = $derived(cards[cardIdx] ?? null);
  const cardKey = (c: MetaCard) => `${c.provider}/${c.subject.id}`;
  /** 这个类型目前有没有能用的下载源（漫画 / 小说只能来自下载源） */
  const noBookSource = $derived((mk === 'manga' || mk === 'novel') && !providers.some(p => supportsKind(p.media_kinds, mk)));

  const PLACEHOLDER: Record<string, string> = {
    anime: '例如：星屑邮局',
    manga: '例如：漫画名或作者',
    novel: '例如：书名或作者',
    music: '例如：林间信号 或 艺人名',
    video: '例如：某个视频的名字',
  };
  const PREFS_HINT: Record<string, string> = {
    anime: '例如：简中字幕、1080p、优先合集',
    manga: '例如：彩色版、高清、优先单行本',
    novel: '例如：完结、带插图',
    music: '例如：无损优先、优先完整专辑',
    video: '例如：1080p、优先完整版',
  };

  onMount(() => {
    loadSubs();
  });

  // 来源或插件有变化时重新读（装了插件就能马上在这里选）
  $effect(() => {
    void plugins.version;
    void em.version;
    void loadProviders();
  });

  async function loadProviders() {
    if (!inTauri) return;
    try {
      [metas, providers] = await Promise.all([metaApi.providers(), fetchApi.providers()]);
    } catch {
      /* 旧版本没有这些命令时忽略 */
    }
    try {
      sources = (await fetchApi.sources()).filter(s => s.enabled);
    } catch {
      /* 忽略 */
    }
  }

  // 搜索范围：不选表示全部启用的来源
  let sources = $state.raw<Source[]>([]);
  let scope = $state<number[]>([]);
  const scoped = $derived(sources.filter(s => supportsKind(s.media_kinds ?? [], mk)));
  const toggleScope = (id: number) => (scope = scope.includes(id) ? scope.filter(x => x !== id) : [...scope, id]);
  const srcOf = (id: number) => sources.find(s => s.id === id);
  /** 出错的来源能做的事：插件来源打开插件详情 */
  function statAction(st: SourceStat): { text: string; run: () => void } | null {
    const act = CODE_TEXT[st.code]?.action;
    const plugin = srcOf(st.source_id)?.plugin;
    if (!act) return null;
    if (act === 'login') return { text: '怎么登录', run: () => toast(CODE_TEXT[st.code].hint) };
    if (!plugin) return null;
    const text = act === 'settings' ? '去填设置' : act === 'enable' ? '去启用' : '看日志';
    return { text, run: () => (plugins.detail = plugin) };
  }

  /** 查作品资料（没有资料源时什么都不做，不影响搜索） */
  async function loadCards(text: string) {
    cards = [];
    cardIdx = 0;
    if (!metas.some(m => supportsKind(m.media_kinds, mk))) return;
    try {
      const r = await metaApi.search(text, mk);
      cards = r.cards;
    } catch {
      /* 资料查不到不影响下载 */
    }
  }

  async function toggleEps(c: MetaCard) {
    const k = cardKey(c);
    if (eps[k]) {
      const next = { ...eps };
      delete next[k];
      eps = next;
      return;
    }
    eps[k] = 'loading';
    try {
      eps[k] = await metaApi.episodes(c.provider, c.subject.id);
    } catch (err) {
      toast(String(err));
      const next = { ...eps };
      delete next[k];
      eps = next;
    }
  }

  async function loadSubs() {
    if (!inTauri) return;
    try {
      subs = await fetchApi.subs();
    } catch {
      /* 没有订阅时忽略 */
    }
  }

  async function doSearch() {
    const text = q.trim();
    if (!text) return toast('先输入要搜索的名字');
    if (!inTauri) return toast('请在桌面程序里搜索');
    searching = true;
    out = null;
    picks = {};
    sel = {};
    qsel = {};
    loadCards(text);
    try {
      out = await fetchApi.search(text, mk, scope.filter(id => scoped.some(s => s.id === id)));
    } catch (err) {
      toast(String(err));
    } finally {
      searching = false;
    }
  }

  async function aiPick() {
    if (!out || out.hits.length === 0) return;
    picking = true;
    try {
      const list = await fetchApi.aiPick(out.hits, prefs);
      const next: Record<number, string> = {};
      for (const p of list) next[p.index] = p.reason || '推荐';
      picks = next;
      toast(list.length ? `AI 推荐了 ${list.length} 个版本` : 'AI 没有挑出合适的版本');
    } catch (err) {
      toast(String(err));
    } finally {
      picking = false;
    }
  }

  function toItem(h: Hit): DlItem {
    return { title: h.title, url: h.url, kind: h.kind, mediaKind: mk, sourceId: h.source_id, size: h.size };
  }

  /** 带上这一行选的档位 */
  function hitAt(i: number): Hit {
    const h = out!.hits[i];
    return qsel[i] !== undefined ? withQuality(h, qsel[i]) : h;
  }

  /** 这一组是不是下载源的结果（按专辑 / 作品分组，“全部下载”下整组） */
  const isProviderGroup = (g: Group) => !!out && out.hits[g.indices[0]]?.kind === 'provider';
  /** 这一组的结果带集数，可以追更（下载源给了 episode 时也可以） */
  const hasEpisodes = (g: Group) => !!out && g.indices.some(i => out!.hits[i].episode !== null);
  /** 下载源结果的说明行：音乐显示艺人专辑，其他显示作品、集数、作者 / 字幕组 */
  function itemLine(it: ProviderItem): string {
    if (it.media_kind === 'music' || (!it.series && it.episode === null)) {
      return [it.artist || (it.media_kind === 'music' ? '艺人未知' : ''), it.album, it.year, fmtDur(it.duration)].filter(Boolean).join(' · ');
    }
    const season = it.season && it.season > 1 ? `第 ${it.season} 季` : '';
    return [it.series, season, epText(it.media_kind, it.episode), it.artist, it.year, fmtDur(it.duration)].filter(Boolean).join(' · ');
  }

  /** 磁力 / 种子先弹确认页（TorrentQueue），勾选后只下载选中的文件 */
  let btQueue = $state<DlItem[]>([]);
  let directCount = 0;
  const isBtItem = (it: DlItem) => it.kind === 'magnet' || it.kind === 'torrent' || (!it.kind && isBtUrl(it.url));

  async function download(hits: Hit[]) {
    if (hits.length === 0) return toast('没有可下载的条目');
    if (btQueue.length) return toast('还有种子在等你确认');
    const items = hits.map(toItem);
    const direct = items.filter(it => !isBtItem(it));
    const bt = items.filter(isBtItem);
    directCount = 0;
    try {
      if (direct.length) {
        directCount = (await fetchApi.download(direct)).length;
        onqueued?.();
      }
    } catch (err) {
      toast(String(err));
    }
    if (bt.length) btQueue = bt;
    else if (directCount) toast(`已加入下载队列：${directCount} 个`);
  }

  function btDone(added: number) {
    btQueue = [];
    const n = added + directCount;
    if (n) toast(`已加入下载队列：${n} 个`);
  }

  /** 整季：优先下载 AI 推荐的版本，没有推荐就下第一个 */
  function seasonHits(g: Group): Hit[] {
    if (!out) return [];
    // 下载源：整张专辑全部下载
    if (isProviderGroup(g)) return g.indices.map(hitAt);
    const recs = g.indices.filter(i => picks[i] !== undefined).map(hitAt);
    if (recs.length) return recs;
    return [hitAt(g.indices[0])];
  }

  function selectedHits(): Hit[] {
    if (!out) return [];
    return out.hits.map((_, i) => i).filter(i => sel[i]).map(hitAt);
  }

  const selCount = $derived(out ? out.hits.filter((_, i) => sel[i]).length : 0);

  function subOf(g: Group): SubRow | undefined {
    if (!out) return undefined;
    const h = out.hits[g.indices[0]];
    return subs.find(s => s.query === g.work && s.source_id === h.source_id);
  }

  async function toggleSub(g: Group) {
    if (!out) return;
    const existing = subOf(g);
    try {
      if (existing) {
        await fetchApi.unsubscribe(existing.id);
        toast('已取消追更');
      } else {
        const h = out.hits[g.indices[0]];
        await fetchApi.subscribe(h.source_id, g.work, mk);
        toast(`已订阅：${g.work}，之后有新集会自动下载`);
      }
      await loadSubs();
    } catch (err) {
      toast(String(err));
    }
  }

  async function unsub(s: SubRow) {
    try {
      await fetchApi.unsubscribe(s.id);
      await loadSubs();
    } catch (err) {
      toast(String(err));
    }
  }
</script>

<div class="card-head">
  <h5>搜索并下载</h5>
</div>

<div class="frow">
  <div class="seg">
    {#each Object.keys(MEDIA_TEXT) as k (k)}
      <button class:on={mk === k} onclick={() => { mk = k; cards = []; }}>{MEDIA_TEXT[k]}</button>
    {/each}
  </div>
  <input class="tin" style="flex:1;min-width:140px" placeholder={PLACEHOLDER[mk]} autocomplete="off" bind:value={q} onkeydown={e => e.key === 'Enter' && doSearch()} />
  <button class="mp-btn" onclick={doSearch} disabled={searching}>{searching ? '搜索中…' : '搜索'}</button>
</div>

{#if scoped.length > 1}
  <div class="scope">
    <span>搜索范围</span>
    <button class="mp-chip" class:on={!scope.some(id => scoped.some(s => s.id === id))} onclick={() => (scope = [])}>全部 {scoped.length} 个</button>
    {#each scoped as s (s.id)}
      <button class="mp-chip" class:on={scope.includes(s.id)} title={s.plugin ? `插件提供的下载源` : ''} onclick={() => toggleScope(s.id)}>{s.name}</button>
    {/each}
  </div>
{/if}

<div class="frow" style="margin-top:10px">
  <input class="tin" style="flex:1;min-width:140px" placeholder={PREFS_HINT[mk]} bind:value={prefs} />
  <button class="mp-btn ghost" onclick={aiPick} disabled={!out || out.hits.length === 0 || picking}>{picking ? 'AI 挑选中…' : 'AI 挑选推荐'}</button>
  <button class="mp-btn ghost" onclick={() => download(selectedHits())} disabled={selCount === 0}>下载选中（{selCount}）</button>
</div>

{#if card}
  {@const sub = card.subject}
  {@const ep = eps[cardKey(card)]}
  <div class="mcard">
    {#if sub.cover}<img class="mcv" src={sub.cover} alt="" loading="lazy" referrerpolicy="no-referrer" />{/if}
    <div class="mbody">
      <div class="mtitle">
        <b>{sub.title_cn || sub.title}</b>
        {#if sub.title_cn && sub.title && sub.title !== sub.title_cn}<small>{sub.title}</small>{/if}
      </div>
      <small>
        {[sub.year ?? '', sub.total ? `共 ${sub.total} ${mk === 'manga' ? '话' : mk === 'novel' ? '章' : '集'}` : '', sub.rating ? `评分 ${sub.rating.toFixed(1)}` : '', sub.staff.slice(0, 2).join('、'), '资料：' + card.provider_name]
          .filter(Boolean)
          .join(' · ')}
      </small>
      {#if sub.summary}<p class="msum">{sub.summary}</p>{/if}
      <div class="frow mrow">
        <button class="mp-btn ghost" onclick={() => toggleEps(card)}>{ep ? '收起分集' : '查看分集'}</button>
        {#if card.link}<a class="mp-btn ghost" href={card.link} target="_blank" rel="noreferrer">作品页</a>{/if}
        {#if cards.length > 1}
          <button class="mp-btn ghost" title="资料对不上时换一个" onclick={() => (cardIdx = (cardIdx + 1) % cards.length)}>不是这部（{cardIdx + 1}/{cards.length}）</button>
        {/if}
      </div>
      {#if ep === 'loading'}
        <small>正在读取分集…</small>
      {:else if ep}
        {#if ep.length === 0}
          <small>这部作品没有分集信息</small>
        {:else}
          <ol class="eps">
            {#each ep as e (e.sort + e.title)}
              <li><span class="badge2">{e.special ? 'SP' : ''}{e.sort}</span> {e.title_cn || e.title || '（无标题）'}{#if e.airdate}<small> · {e.airdate}</small>{/if}</li>
            {/each}
          </ol>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<div style="margin-top:12px">
  {#if noBookSource && !out && !searching}
    <div class="note" style="margin-bottom:10px">
      还没有能搜{MEDIA_TEXT[mk]}的下载源。可以安装提供{MEDIA_TEXT[mk]}下载源的插件，或添加你自己的网盘 / NAS 目录。
      <div style="margin-top:6px"><button class="mp-btn ghost" onclick={pickPlugin}>安装插件…</button></div>
    </div>
  {/if}
  {#if searching}
    <div class="empty">正在搜索你添加的所有来源…</div>
  {:else if !out}
    <div class="empty">输入名字后点“搜索”。会同时搜索左边所有来源，结果按作品分组，可以用 AI 挑出推荐的版本。</div>
  {:else}
    {#if out.stats?.length}
      <div class="stats">
        {#each out.stats.filter(st => !st.skipped) as st (st.source_id)}
          {@const a = statAction(st)}
          <span class="stat" class:bad={!!st.error} title={st.error ? `${st.error}${CODE_TEXT[st.code] ? '\n' + CODE_TEXT[st.code].hint : ''}` : `${st.ms} 毫秒`}>
            {st.name}：{st.error ? (CODE_TEXT[st.code]?.label ?? '出错') : `${st.count} 条`}
            {#if a}<button class="linkbtn" onclick={a.run}>{a.text}</button>{/if}
          </span>
        {/each}
      </div>
      {#if out.stats.some(st => st.error)}
        <div class="note" style="margin-bottom:10px">{#each out.stats.filter(st => st.error) as st (st.source_id)}<div>{st.name}：{st.error}</div>{/each}</div>
      {/if}
    {:else if out.notes.length > 0}
      <div class="note" style="margin-bottom:10px">{#each out.notes as n (n)}<div>{n}</div>{/each}</div>
    {/if}
    {#if out.hits.length === 0}
      <div class="empty">没有找到结果。可以换个写法再试，或检查来源是否可用。</div>
    {/if}
    {#each out.groups as g (g.work + g.indices[0])}
      <section class="hitgrp">
        <div class="grp-head">
          <b>{g.work}</b>
          <span class="badge2">{g.indices.length} 个版本</span>
          {#if subOf(g)}<span class="badge2 ok">已追更</span>{/if}
          <span class="spacer"></span>
          {#if isProviderGroup(g)}
            <button class="mp-btn ghost" onclick={() => download(seasonHits(g))}>全部下载</button>
            {#if hasEpisodes(g)}
              <button class="mp-btn ghost" onclick={() => toggleSub(g)}>{subOf(g) ? '取消追更' : '追更'}</button>
            {/if}
          {:else}
            <button class="mp-btn ghost" onclick={() => download(seasonHits(g))}>下载整季</button>
            <button class="mp-btn ghost" onclick={() => toggleSub(g)}>{subOf(g) ? '取消追更' : '追更'}</button>
          {/if}
        </div>
        {#each g.indices as i (i)}
          {@const h = out.hits[i]}
          {@const it = items[i]}
          <div class="rs" class:on={picks[i] !== undefined || sel[i]}>
            <div class="rs-top">
              <input type="checkbox" class="mp-sw" aria-label="选中" bind:checked={sel[i]} />
              {#if it?.cover}<img class="hitcover" src={it.cover} alt="" loading="lazy" referrerpolicy="no-referrer" />{/if}
              <b>{it ? it.title : h.title}</b>
              {#if picks[i] !== undefined}<span class="badge2 ai">推荐</span>{/if}
              {#if h.rule_updated}<span class="badge2">规则已更新</span>{/if}
            </div>
            {#if it}
              {@const qk = qsel[i] ?? providerQualityKey(h)}
              {@const cur = it.qualities.find(x => x.key === qk) ?? it.qualities[0]}
              <small>{itemLine(it)}</small>
              <div class="qrow">
                {#if it.qualities.length > 1}
                  <select class="sel" aria-label={mk === 'music' ? '音质' : '版本'} value={qk} onchange={e => (qsel[i] = e.currentTarget.value)}>
                    {#each it.qualities as x (x.key)}
                      <option value={x.key}>{x.label || x.format.toUpperCase()}{x.size ? ' · ' + fmtBytes(x.size) : ''}</option>
                    {/each}
                  </select>
                {:else if cur}
                  <span class="badge2">{cur.label || cur.format.toUpperCase()}</span>
                {/if}
                <small>{cur?.format ? cur.format.toUpperCase() : ''}{cur?.bitrate ? ' ' + cur.bitrate + 'k' : ''} · {fmtBytes(cur?.size ?? h.size)} · {hitKindText(h.kind)}</small>
              </div>
            {:else}
              <small>{h.resolution ?? '清晰度未知'} · {h.group ?? '字幕组未知'} · {fmtBytes(h.size)} · {hitKindText(h.kind)}{h.date ? ' · ' + h.date : ''}</small>
            {/if}
            <small>来源：{h.source_name}{picks[i] !== undefined ? ' · ' + picks[i] : ''}</small>
            <div class="acts">
              <button class="mp-btn ghost" onclick={() => download([hitAt(i)])}>下载这一个</button>
            </div>
          </div>
        {/each}
      </section>
    {/each}
  {/if}
</div>

{#if subs.length > 0}
  <div class="subs">
    <h5>我的追更</h5>
    <div class="chips">
      {#each subs as s (s.id)}
        <span class="mp-chip">{s.query} · {s.source_name}
          <button class="x" aria-label="取消追更" onclick={() => unsub(s)}>×</button></span>
      {/each}
    </div>
    <div class="note" style="margin-top:8px">追更每 30 分钟检查一次，新出现的集会自动加入下载队列。</div>
  </div>
{/if}

{#if btQueue.length}
  <TorrentQueue items={btQueue} ondone={btDone} {onqueued} />
{/if}

<style>
  .scope {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    align-items: center;
    margin-top: 8px;
    font-size: 12.5px;
  }
  .scope > span {
    color: var(--mp-sub);
  }
  .stats {
    display: flex;
    gap: 6px 12px;
    flex-wrap: wrap;
    font-size: 12px;
    color: var(--mp-sub);
    margin-bottom: 8px;
  }
  .stat.bad {
    color: #ff8a8a;
  }
  .linkbtn {
    background: none;
    border: 0;
    padding: 0 0 0 4px;
    color: var(--mp-accent, #7aa7ff);
    cursor: pointer;
    font: inherit;
  }
  .card-head h5,
  .subs h5 {
    margin: 0 0 8px;
  }
  .spacer {
    flex: 1;
  }
  /* 不要用 .grp / .cv：全局样式里已有同名类（侧栏分组标题、设置页网格），会把结果组排成横向 flex */
  .hitgrp {
    display: block;
    margin-bottom: 14px;
    min-width: 0;
  }
  .grp-head b {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hitgrp .mp-btn {
    white-space: nowrap;
    flex: none;
  }
  .qrow .sel {
    max-width: 100%;
  }
  .grp-head {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
    margin-bottom: 6px;
    font-size: 13.5px;
  }
  .rs-top {
    grid-column: 1 / -1;
    display: flex;
    gap: 8px;
    align-items: center;
    min-width: 0;
  }
  .rs-top b {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rs small {
    grid-column: 1 / -1;
  }
  .hitcover {
    width: 34px;
    height: 34px;
    border-radius: 6px;
    object-fit: cover;
    flex: none;
  }
  .qrow {
    grid-column: 1 / -1;
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .qrow small {
    grid-column: auto;
  }
  .mcard {
    display: flex;
    gap: 12px;
    margin-top: 12px;
    padding: 10px;
    border-radius: 10px;
    background: var(--mp-hover, rgba(127, 127, 127, 0.08));
    min-width: 0;
  }
  .mcv {
    width: 72px;
    height: 100px;
    border-radius: 8px;
    object-fit: cover;
    flex: none;
  }
  .mbody {
    display: grid;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }
  .mtitle {
    display: flex;
    gap: 8px;
    align-items: baseline;
    flex-wrap: wrap;
  }
  .msum {
    margin: 2px 0;
    font-size: 12.5px;
    line-height: 1.5;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .mrow {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .eps {
    margin: 4px 0 0;
    padding: 0;
    list-style: none;
    max-height: 180px;
    overflow: auto;
    font-size: 12.5px;
    display: grid;
    gap: 3px;
  }
  .chips {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .x {
    background: none;
    border: 0;
    color: inherit;
    margin-left: 4px;
    cursor: pointer;
  }
</style>
