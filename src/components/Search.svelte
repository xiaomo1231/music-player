<script lang="ts">
  // 顶部搜索框：本地媒体库马上出结果；停顿一下再去搜已启用的音乐下载源（插件），结果可以直接下载进音乐库
  import { onMount } from 'svelte';
  import Icon from '../lib/Icon.svelte';
  import Cover from './Cover.svelte';
  import { invoke, inTauri, listen, type Hit } from '../lib/api';
  import { ui, toast } from '../lib/ui.svelte';
  import { lib } from '../lib/library.svelte';
  import { player } from '../lib/player.svelte';
  import {
    fetchApi,
    findInLibrary,
    fmtDur,
    gsProvidersOn,
    providerItem,
    providerKey,
    STATUS_TEXT,
    supportsKind,
    type Hit as FetchHit,
    type TaskRow,
  } from '../lib/fetch';

  let q = $state('');
  let hits = $state<Hit[]>([]);
  let open = $state(false);
  let input: HTMLInputElement;
  let t: ReturnType<typeof setTimeout> | undefined;
  let localSeq = 0;
  const LABEL: Record<string, string> = { music: '歌曲', anime: '番剧', video: '视频', manga: '漫画', novel: '小说' };
  const groups = $derived(Object.keys(LABEL).map(k => [k, hits.filter(h => h.kind === k)] as const).filter(g => g[1].length));

  // ---------- 下载源 ----------
  /** 停顿多久再搜下载源（联网，比本地慢，避免每敲一个字就搜一次） */
  const REMOTE_DELAY = 550;
  const REMOTE_MAX = 8;
  let rt: ReturnType<typeof setTimeout> | undefined;
  let rseq = 0;
  /** 这次参与搜索的下载源个数（0 = 没有可用的下载源 / 开关关了） */
  let rcount = $state(0);
  let rbusy = $state(false);
  let rhits = $state.raw<FetchHit[]>([]);
  /** rhits 对应的关键词（输入变了以后旧结果不再显示） */
  let rq = $state('');
  /** 同一个关键词短时间内不重复联网 */
  const rcache = new Map<string, { at: number; hits: FetchHit[]; count: number }>();
  const CACHE_MS = 5 * 60_000;
  /** 下载任务：下载源条目身份 → 任务（显示排队 / 进度 / 已入库） */
  let tasks = $state<Record<string, TaskRow>>({});
  let tasksLoaded = false;

  const showRemote = $derived(q.trim() === rq && (rbusy || rhits.length > 0));
  const remoteList = $derived(showRemote ? rhits.slice(0, REMOTE_MAX) : []);

  /** 中文一个字就能搜；字母数字至少两个 */
  const enoughForRemote = (s: string) => s.length >= 2 || /[^\x00-\x7f]/.test(s);

  function onInput() {
    clearTimeout(t);
    const lseq = ++localSeq;
    t = setTimeout(async () => {
      if (!q.trim() || !inTauri) { hits = []; return; }
      // 打字快时前一次查询可能比后一次晚回来：只用最新一次的结果
      const res = await invoke<Hit[]>('search', { q }).catch(() => null);
      if (lseq !== localSeq || !res) return;
      hits = res;
      open = true;
    }, 140);
    clearTimeout(rt);
    const text = q.trim();
    if (!text || !inTauri) { rhits = []; rq = ''; rbusy = false; rseq++; return; }
    rt = setTimeout(() => void searchRemote(text), REMOTE_DELAY);
  }

  async function searchRemote(text: string) {
    if (!gsProvidersOn() || !enoughForRemote(text)) { rhits = []; rq = text; rbusy = false; return; }
    const seq = ++rseq;
    const cached = rcache.get(text);
    if (cached && Date.now() - cached.at < CACHE_MS) {
      rhits = cached.hits; rcount = cached.count; rq = text; rbusy = false;
      if (cached.hits.length) open = document.activeElement === input;
      return;
    }
    let ids: number[];
    try {
      const sources = await fetchApi.sources();
      ids = sources.filter(s => s.enabled && s.kind === 'provider' && supportsKind(s.media_kinds ?? [], 'music')).map(s => s.id);
    } catch {
      ids = [];
    }
    if (seq !== rseq) return;
    rcount = ids.length;
    if (!ids.length) { rhits = []; rq = text; rbusy = false; return; }
    rq = text;
    rhits = [];
    rbusy = true;
    void loadTasks();
    try {
      // works = false：搜单曲，不折叠成专辑 / 作品卡
      const out = await fetchApi.search(text, 'music', ids, false);
      if (seq !== rseq) return;
      const list = out.hits.filter(h => h.kind === 'provider' && !h.expand);
      rcache.set(text, { at: Date.now(), hits: list, count: ids.length });
      rhits = list;
      if (list.length) open = document.activeElement === input;
    } catch {
      if (seq === rseq) rhits = [];
    } finally {
      if (seq === rseq) rbusy = false;
    }
  }

  async function loadTasks() {
    if (tasksLoaded) return;
    tasksLoaded = true;
    try {
      const next: Record<string, TaskRow> = {};
      for (const row of await fetchApi.tasks()) {
        const k = providerKey(row.url);
        // 列表是新的在前：同一首只记最新的任务
        if (k && !next[k]) next[k] = row;
      }
      tasks = { ...next, ...tasks };
    } catch {
      tasksLoaded = false;
    }
  }

  onMount(() => {
    if (!inTauri) return;
    let off: (() => void) | undefined;
    let dead = false;
    listen<TaskRow>('fetch://progress', row => {
      const k = providerKey(row.url);
      if (k) tasks[k] = row;
    }).then(f => (dead ? f() : (off = f)));
    return () => { dead = true; off?.(); };
  });

  type RowState = { text: string; cls: string; act?: 'download' | 'retry' | 'play' };
  function rowState(h: FetchHit): RowState {
    const k = providerKey(h.url);
    const task = k ? tasks[k] : undefined;
    const it = providerItem(h);
    if (task && task.status !== 'canceled') {
      if (task.status === 'failed') return { text: '失败，重试', cls: 'bad', act: 'retry' };
      if (task.status === 'done') return { text: '已入库', cls: 'ok', act: localTrack(h) ? 'play' : undefined };
      if (task.status === 'downloading') return { text: `下载中 ${Math.round(Math.min(1, Math.max(0, task.progress)) * 100)}%`, cls: 'run' };
      return { text: STATUS_TEXT[task.status] ?? task.status, cls: 'run' };
    }
    if (it && localTrack(h)) return { text: '已在库', cls: 'ok', act: 'play' };
    return { text: '下载', cls: '', act: 'download' };
  }

  function localTrack(h: FetchHit) {
    const it = providerItem(h);
    return findInLibrary(lib.tracks, it?.title || h.title, it?.artist ?? '');
  }

  async function act(h: FetchHit, e: MouseEvent) {
    // 不让输入框失焦，下拉框保持打开，可以接着下别的
    e.preventDefault();
    e.stopPropagation();
    const st = rowState(h);
    if (st.act === 'play') {
      const tr = localTrack(h);
      if (tr) { open = false; player.playList([tr], 0, 'search'); ui.go('now'); }
      return;
    }
    const k = providerKey(h.url);
    if (st.act === 'retry' && k && tasks[k]) {
      try { await fetchApi.retry(tasks[k].id); } catch (err) { toast(String(err)); }
      return;
    }
    if (st.act !== 'download') return;
    try {
      const [id] = await fetchApi.download([{ title: h.title, url: h.url, kind: h.kind, mediaKind: 'music', sourceId: h.source_id, size: h.size }]);
      if (k && id !== undefined && !tasks[k]) {
        tasks[k] = { id, source_id: h.source_id, title: h.title, kind: h.kind, url: h.url, status: 'queued', progress: 0, speed: 0, size: h.size ?? 0, save_path: '', error: '', created_at: Date.now() / 1000, finished_at: null, media_kind: 'music' };
      }
      toast(`已加入下载：${h.title}，下完会自动整理进音乐库`);
    } catch (err) {
      toast(String(err));
    }
  }

  function seeAll(e: MouseEvent) {
    e.preventDefault();
    open = false;
    ui.fetchQuery = { q: q.trim(), mk: 'music' };
    ui.go('get');
  }

  function subOf(h: FetchHit): string {
    const it = providerItem(h);
    return [it?.artist, it?.album, h.source_name].filter(Boolean).join(' · ');
  }

  // ---------- 本地结果 ----------
  function pick(h: Hit) {
    open = false;
    if (h.kind === 'music') {
      const tr = lib.byId(h.id);
      if (tr) { player.playList([tr], 0, 'search'); ui.go('now'); }
    } else if (h.kind === 'anime' && h.series_id) { ui.seriesId = h.series_id; ui.go('anime'); }
    else if (h.series_id) { ui.seriesId = h.series_id; ui.go('video'); }
    else ui.openItem(h.id, h.kind);
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Enter' && hits[0]) pick(hits[0]);
    if (e.key === 'Escape') { open = false; (e.target as HTMLInputElement).blur(); e.stopPropagation(); }
  }
</script>
<label class="search"><Icon name="search" />
  <input id="gs" bind:this={input} bind:value={q} oninput={onInput} onkeydown={key} onfocus={() => (open = hits.length > 0 || remoteList.length > 0)} onblur={() => setTimeout(() => (open = false), 150)}
    placeholder="搜索歌曲、艺人、番剧、漫画、小说…（支持拼音首字母）" autocomplete="off" /><kbd>/</kbd></label>
<div class="results" class:open={open && q.trim().length > 0}>
  {#if !hits.length && !showRemote}<h6>没有找到“{q}”</h6>{/if}
  {#each groups as [k, list]}
    <h6>{LABEL[k]} · {list.length}</h6>
    {#each list.slice(0, 8) as h (h.id)}
      <div role="button" tabindex="-1" onmousedown={() => pick(h)}><Cover tag="i" cls="" cover={h.cover} name={h.title} />{h.title}<small>{h.sub}</small></div>
    {/each}
  {/each}
  {#if showRemote}
    <h6 class="rhead">
      <span>下载源{rhits.length ? ` · ${rhits.length}` : ''}</span>
      {#if rbusy}<em>正在搜 {rcount} 个下载源…</em>{/if}
      <button class="more" onmousedown={seeAll}>查看全部</button>
    </h6>
    {#each remoteList as h (h.url)}
      {@const it = providerItem(h)}
      {@const st = rowState(h)}
      <div class="rrow" role="presentation">
        {#if it?.cover}<img src={it.cover} alt="" loading="lazy" referrerpolicy="no-referrer" />{:else}<Cover tag="i" cls="" name={h.title} />{/if}
        <span class="rt"><b>{it?.title || h.title}</b><small>{subOf(h)}</small></span>
        {#if it?.duration}<small class="dur">{fmtDur(it.duration)}</small>{/if}
        {#if st.act}
          <button class="rbtn {st.cls}" title={st.act === 'play' ? '播放本地这首' : st.act === 'retry' ? '重新下载' : '下载到本地并整理进音乐库'} onmousedown={e => act(h, e)}>
            {#if st.act === 'download'}<Icon name="download" />{:else if st.act === 'play'}<Icon name="play" />{/if}{st.text}
          </button>
        {:else}
          <span class="rst {st.cls}">{st.text}</span>
        {/if}
      </div>
    {/each}
  {/if}
</div>

<style>
  .rhead { display: flex; align-items: center; gap: 8px; }
  .rhead em { font-style: normal; color: var(--mp-sub); letter-spacing: 0; }
  .rhead .more { margin-left: auto; background: none; border: 0; color: var(--mp-accent, inherit); font-size: 11px; cursor: pointer; padding: 0; letter-spacing: 0; }
  .rhead .more:hover { text-decoration: underline; }
  .results .rrow { cursor: default; }
  .rrow img { width: 28px; height: 28px; border-radius: 6px; object-fit: cover; flex: none; }
  .rt { display: flex; flex-direction: column; min-width: 0; flex: 1; }
  .rt b { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .results .rt small { margin-left: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .results .rrow small.dur { margin-left: 0; flex: none; }
  .rbtn, .rst { flex: none; display: inline-flex; align-items: center; gap: 4px; font-size: 12px; padding: 4px 10px; border-radius: 999px; white-space: nowrap; }
  .rbtn { border: 1px solid var(--edge); background: rgba(255, 255, 255, 0.06); color: inherit; cursor: pointer; }
  .rbtn:hover { background: rgba(255, 255, 255, 0.14); }
  .rbtn :global(svg) { width: 13px; height: 13px; }
  .rst { color: var(--mp-sub); }
  .ok { color: #6fd39a; }
  .bad { color: #f08a8a; }
  .run { color: var(--mp-sub); }
</style>
