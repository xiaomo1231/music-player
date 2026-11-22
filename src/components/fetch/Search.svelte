<script lang="ts">
  // 搜索并下载：同时搜所有来源，按作品分组；AI 挑推荐版本；整季 / 选中下载；追更订阅
  import { onMount } from 'svelte';
  import { inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import {
    fetchApi,
    fmtBytes,
    fmtDur,
    hitKindText,
    MEDIA_TEXT,
    providerItem,
    providerQualityKey,
    withQuality,
    type DlItem,
    type Group,
    type Hit,
    type ProviderItem,
    type SearchOut,
    type SubRow,
  } from '../../lib/fetch';

  let { onqueued }: { onqueued?: () => void } = $props();

  let mk = $state<'anime' | 'music' | 'video'>('anime');
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

  const PLACEHOLDER: Record<string, string> = {
    anime: '例如：星屑邮局',
    music: '例如：林间信号 或 艺人名',
    video: '例如：某个视频的名字',
  };
  const PREFS_HINT: Record<string, string> = {
    anime: '例如：简中字幕、1080p、优先合集',
    music: '例如：无损优先、优先完整专辑',
    video: '例如：1080p、优先完整版',
  };

  onMount(() => {
    loadSubs();
  });

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
    try {
      out = await fetchApi.search(text, mk);
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

  /** 这一组是不是下载源的结果（按专辑分组，没有“集”，不能追更） */
  const isProviderGroup = (g: Group) => !!out && out.hits[g.indices[0]]?.kind === 'provider';

  async function download(hits: Hit[]) {
    if (hits.length === 0) return toast('没有可下载的条目');
    try {
      const ids = await fetchApi.download(hits.map(toItem));
      toast(`已加入下载队列：${ids.length} 个`);
      onqueued?.();
    } catch (err) {
      toast(String(err));
    }
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
      <button class:on={mk === k} onclick={() => (mk = k as typeof mk)}>{MEDIA_TEXT[k]}</button>
    {/each}
  </div>
  <input class="tin" style="flex:1;min-width:140px" placeholder={PLACEHOLDER[mk]} autocomplete="off" bind:value={q} onkeydown={e => e.key === 'Enter' && doSearch()} />
  <button class="mp-btn" onclick={doSearch} disabled={searching}>{searching ? '搜索中…' : '搜索'}</button>
</div>

<div class="frow" style="margin-top:10px">
  <input class="tin" style="flex:1;min-width:140px" placeholder={PREFS_HINT[mk]} bind:value={prefs} />
  <button class="mp-btn ghost" onclick={aiPick} disabled={!out || out.hits.length === 0 || picking}>{picking ? 'AI 挑选中…' : 'AI 挑选推荐'}</button>
  <button class="mp-btn ghost" onclick={() => download(selectedHits())} disabled={selCount === 0}>下载选中（{selCount}）</button>
</div>

<div style="margin-top:12px">
  {#if searching}
    <div class="empty">正在搜索你添加的所有来源…</div>
  {:else if !out}
    <div class="empty">输入名字后点“搜索”。会同时搜索左边所有来源，结果按作品分组，可以用 AI 挑出推荐的版本。</div>
  {:else}
    {#if out.notes.length > 0}
      <div class="note" style="margin-bottom:10px">{#each out.notes as n (n)}<div>{n}</div>{/each}</div>
    {/if}
    {#if out.hits.length === 0}
      <div class="empty">没有找到结果。可以换个写法再试，或检查来源是否可用。</div>
    {/if}
    {#each out.groups as g (g.work + g.indices[0])}
      <section class="grp">
        <div class="grp-head">
          <b>{g.work}</b>
          <span class="badge2">{g.indices.length} 个版本</span>
          {#if subOf(g)}<span class="badge2 ok">已追更</span>{/if}
          <span class="spacer"></span>
          {#if isProviderGroup(g)}
            <button class="mp-btn ghost" onclick={() => download(seasonHits(g))}>全部下载</button>
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
              {#if it?.cover}<img class="cv" src={it.cover} alt="" loading="lazy" referrerpolicy="no-referrer" />{/if}
              <b>{it ? it.title : h.title}</b>
              {#if picks[i] !== undefined}<span class="badge2 ai">推荐</span>{/if}
              {#if h.rule_updated}<span class="badge2">规则已更新</span>{/if}
            </div>
            {#if it}
              {@const qk = qsel[i] ?? providerQualityKey(h)}
              {@const cur = it.qualities.find(x => x.key === qk) ?? it.qualities[0]}
              <small>{[it.artist || '艺人未知', it.album, it.year, fmtDur(it.duration)].filter(Boolean).join(' · ')}</small>
              <div class="qrow">
                {#if it.qualities.length > 1}
                  <select class="sel" aria-label="音质" value={qk} onchange={e => (qsel[i] = e.currentTarget.value)}>
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

<style>
  .card-head h5,
  .subs h5 {
    margin: 0 0 8px;
  }
  .spacer {
    flex: 1;
  }
  .grp {
    margin-bottom: 14px;
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
  .cv {
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
