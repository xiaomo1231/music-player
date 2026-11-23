<script lang="ts">
  // 种子 / 磁力下载前的确认页：先读种子信息（磁力要先从网络拿元数据），列出里面的文件、体积和识别结果，勾选后只下载选中的
  import Modal from '../Modal.svelte';
  import { fetchApi, fmtBytes, MEDIA_TEXT, taskTitleOf, type TorrentFile, type TorrentInfo } from '../../lib/fetch';

  const KINDS = ['video', 'anime', 'music'];

  let {
    url,
    mediaKind,
    onconfirm,
    onfallback,
    onskip,
  }: {
    url: string;
    mediaKind: string;
    /** 确认下载：选中的文件（全选时为 null）和归类 */
    onconfirm: (info: TorrentInfo, files: number[] | null, size: number, mediaKind: string) => void;
    /** 读不到种子信息时仍然整个下载 */
    onfallback: (mediaKind: string) => void;
    onskip: () => void;
  } = $props();

  let info = $state<TorrentInfo | null>(null);
  let err = $state('');
  let picked = $state<Record<number, boolean>>({});
  // svelte-ignore state_referenced_locally
  let mk = $state(mediaKind);
  let collapsed = $state<Record<string, boolean>>({});
  let seconds = $state(0);

  $effect(() => {
    const u = url;
    let stale = false;
    info = null;
    err = '';
    seconds = 0;
    const timer = setInterval(() => seconds++, 1000);
    fetchApi
      .inspectTorrent(u)
      .then(r => {
        if (stale) return;
        info = r;
        picked = Object.fromEntries(r.files.map(f => [f.index, f.suggested]));
        // 一个推荐的都没有（比如全是压缩包）时默认全选
        if (!r.files.some(f => f.suggested)) picked = Object.fromEntries(r.files.map(f => [f.index, true]));
        // 识别出集数的多半是番剧 / 剧集
        if (r.episodes >= 2 && mk === 'video') mk = 'anime';
      })
      .catch(e => {
        if (!stale) err = String(e);
      })
      .finally(() => clearInterval(timer));
    return () => {
      stale = true;
      clearInterval(timer);
    };
  });

  const isMagnet = $derived(url.trim().toLowerCase().startsWith('magnet:'));

  /** 按文件夹分组（种子里的相对路径），文件夹内按路径排序 */
  const groups = $derived.by(() => {
    if (!info) return [];
    const m = new Map<string, TorrentFile[]>();
    for (const f of [...info.files].sort((a, b) => a.path.localeCompare(b.path, 'zh-CN', { numeric: true }))) {
      const i = f.path.lastIndexOf('/');
      const dir = i < 0 ? '' : f.path.slice(0, i);
      if (!m.has(dir)) m.set(dir, []);
      m.get(dir)!.push(f);
    }
    // 根目录的文件在前，子文件夹（SPs、Scans 之类）在后
    return [...m.entries()]
      .sort(([a], [b]) => (a === '' ? -1 : b === '' ? 1 : a.localeCompare(b, 'zh-CN', { numeric: true })))
      .map(([dir, files]) => ({ dir, files, size: files.reduce((s, f) => s + f.size, 0) }));
  });

  const chosen = $derived(info ? info.files.filter(f => picked[f.index]) : []);
  const chosenSize = $derived(chosen.reduce((s, f) => s + f.size, 0));
  const allPicked = $derived(!!info && chosen.length === info.files.length);

  const baseName = (p: string) => p.slice(p.lastIndexOf('/') + 1);

  function setWhere(pred: (f: TorrentFile) => boolean) {
    if (!info) return;
    picked = Object.fromEntries(info.files.map(f => [f.index, pred(f)]));
  }

  function toggleGroup(files: TorrentFile[]) {
    const on = !files.every(f => picked[f.index]);
    for (const f of files) picked[f.index] = on;
  }

  const KIND_CLASS: Record<string, string> = { video: 'v', audio: 'a', sub: 's' };

  function confirm() {
    if (!info || chosen.length === 0) return;
    onconfirm(
      info,
      allPicked ? null : chosen.map(f => f.index),
      chosenSize,
      mk,
    );
  }
</script>

<Modal title={info ? '确认下载内容' : '读取种子信息'} onclose={onskip} wide>
  {#if !info && !err}
    <p class="mp-desc">
      {#if isMagnet}正在通过磁力链接获取种子信息（已等 {seconds} 秒，最多 2 分钟），拿到后会列出里面的文件给你挑选。{:else}正在读取种子文件…{/if}
    </p>
    <div class="wait"><span class="spin"></span><span class="ell">{taskTitleOf(url)}</span></div>
    <div class="foot">
      <span class="spacer"></span>
      <button class="mp-btn ghost" onclick={onskip}>取消</button>
    </div>
  {:else if err}
    <p class="mp-desc">没能读取种子信息：{err}</p>
    <div class="foot">
      <span class="spacer"></span>
      <button class="mp-btn ghost" onclick={onskip}>跳过</button>
      <button class="mp-btn ghost" onclick={() => onfallback(mk)}>仍然下载全部</button>
    </div>
  {:else if info}
    <div class="sum">
      <div class="name" title={info.name}>{info.name}</div>
      <div class="facts">
        {#if info.work && info.work !== info.name}<span>识别为《{info.work}》</span>{/if}
        {#if info.episodes}<span>正片 {info.episodes} 集</span>{/if}
        {#if info.specials}<span>特典 {info.specials} 个</span>{/if}
        <span>共 {info.files.length} 个文件</span>
        <span>总大小 {fmtBytes(info.totalSize)}</span>
      </div>
    </div>

    <div class="quick">
      <button class="mp-chip" class:on={allPicked} onclick={() => setWhere(() => true)}>全选</button>
      <button class="mp-chip" onclick={() => setWhere(f => f.suggested)}>视频和字幕</button>
      {#if info.episodes}
        <button class="mp-chip" onclick={() => setWhere(f => (f.kind === 'video' || f.kind === 'sub') && !f.special)}>只要正片</button>
      {/if}
      <button class="mp-chip" onclick={() => setWhere(() => false)}>全不选</button>
    </div>

    <div class="list">
      {#each groups as g (g.dir)}
        {#if g.dir}
          {@const on = g.files.every(f => picked[f.index])}
          {@const some = !on && g.files.some(f => picked[f.index])}
          <div class="row dir">
            <input type="checkbox" checked={on} indeterminate={some} onchange={() => toggleGroup(g.files)} aria-label="选择文件夹" />
            <button class="dname" onclick={() => (collapsed[g.dir] = !collapsed[g.dir])} title={g.dir}>
              <span class="caret" class:shut={collapsed[g.dir]}>▾</span><span class="ell">{g.dir}</span>
            </button>
            <span class="cnt">{g.files.length} 个</span>
            <span class="size">{fmtBytes(g.size)}</span>
          </div>
        {/if}
        {#if !collapsed[g.dir]}
          {#each g.files as f (f.index)}
            <label class="row" class:indent={!!g.dir} class:off={!picked[f.index]}>
              <input type="checkbox" bind:checked={picked[f.index]} />
              <span class="ell fname" title={f.path}>{baseName(f.path)}</span>
              <span class="tag {KIND_CLASS[f.kind] ?? ''}" class:sp={f.special}>{f.label}</span>
              <span class="size">{fmtBytes(f.size)}</span>
            </label>
          {/each}
        {/if}
      {/each}
    </div>

    <div class="foot">
      <span>下完归到</span>
      <select class="sel" bind:value={mk} aria-label="媒体类型">
        {#each KINDS as k (k)}
          <option value={k}>{MEDIA_TEXT[k]}</option>
        {/each}
      </select>
      <span class="spacer"></span>
      <span class="picked">已选 {chosen.length} / {info.files.length} 个 · {fmtBytes(chosenSize)}</span>
      <button class="mp-btn ghost" onclick={onskip}>取消</button>
      <button class="mp-btn" onclick={confirm} disabled={chosen.length === 0}>下载选中</button>
    </div>
  {/if}
</Modal>

<style>
  .wait {
    display: flex;
    gap: 10px;
    align-items: center;
    font-size: 13px;
    padding: 10px 0;
    min-width: 0;
  }
  .spin {
    flex: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid var(--mp-line);
    border-top-color: var(--mp-accent);
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .sum {
    margin: 6px 0 10px;
  }
  .name {
    font-weight: 600;
    font-size: 14px;
    word-break: break-all;
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    font-size: 12.5px;
    color: var(--mp-sub);
    margin-top: 4px;
  }
  .quick {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 8px;
  }
  .quick .mp-chip {
    padding: 4px 11px;
    font-size: 12px;
    color: inherit;
  }
  .list {
    max-height: 46vh;
    overflow: auto;
    border: 1px solid var(--mp-line);
    border-radius: 10px;
    padding: 4px 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 12px;
    font-size: 12.5px;
    min-width: 0;
    cursor: pointer;
  }
  .row:hover {
    background: rgba(255, 255, 255, 0.05);
  }
  .row.indent {
    padding-left: 34px;
  }
  .row.off .fname {
    opacity: 0.55;
  }
  .row.dir {
    font-weight: 600;
    cursor: default;
  }
  .dname {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 4px;
    align-items: center;
    background: none;
    border: 0;
    color: inherit;
    font: inherit;
    padding: 0;
    text-align: left;
    cursor: pointer;
  }
  .caret {
    display: inline-block;
    transition: transform 0.15s;
  }
  .caret.shut {
    transform: rotate(-90deg);
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .fname {
    flex: 1;
  }
  .cnt {
    color: var(--mp-sub);
    font-weight: 400;
  }
  .size {
    flex: none;
    width: 70px;
    text-align: right;
    color: var(--mp-sub);
    font-variant-numeric: tabular-nums;
  }
  .tag {
    flex: none;
    font-size: 11px;
    padding: 2px 7px;
    border-radius: 5px;
    background: rgba(255, 255, 255, 0.1);
    color: var(--mp-sub);
  }
  .tag.v {
    background: color-mix(in srgb, var(--mp-accent) 22%, transparent);
    color: var(--mp-text);
  }
  .tag.s {
    background: rgba(120, 200, 140, 0.18);
    color: var(--mp-text);
  }
  .tag.a {
    background: rgba(200, 160, 255, 0.18);
    color: var(--mp-text);
  }
  .tag.sp {
    background: rgba(255, 190, 90, 0.2);
    color: var(--mp-text);
  }
  .foot {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
    margin-top: 14px;
    font-size: 12.5px;
    color: var(--mp-sub);
  }
  .picked {
    font-variant-numeric: tabular-nums;
  }
  .spacer {
    flex: 1;
  }
</style>
