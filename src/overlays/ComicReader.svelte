<script lang="ts">
  import { tick } from 'svelte';
  import { invoke, inTauri, fileUrl, comicPage, type ComicItem } from '../lib/api';
  import { ui, toast } from '../lib/ui.svelte';
  import { typing, clamp } from '../lib/util';
  import TranslatePanel from '../components/mangatr/TranslatePanel.svelte';
  import PageOverlay from '../components/mangatr/PageOverlay.svelte';

  type Mode = 'single' | 'double' | 'scroll';
  type Tr = 'orig' | 'tr' | 'both';

  let comics = $state.raw<ComicItem[]>([]);
  let item = $state.raw<ComicItem | null>(null);
  let err = $state('');
  let loading = $state(false);
  let page = $state(0);
  let mode = $state<Mode>('single');
  let rtl = $state(true);
  let zoom = $state(1);
  let aiOpen = $state(false);
  let trMode = $state<Tr>('orig');
  let pagesEl: HTMLDivElement | undefined = $state();

  /** 当前加载的是哪一本（非响应式，给保存进度用） */
  let loadedId = 0;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let clickTimer: ReturnType<typeof setTimeout> | undefined;
  let lastWheel = 0;
  let endNotified = false;

  const isPdf = $derived(item?.format === 'pdf');
  const count = $derived(item ? item.pages : 0);
  const pageIdx = $derived(Array.from({ length: count }, (_, i) => i));
  const nextItem = $derived.by<ComicItem | null>(() => {
    if (!item) return null;
    const idx = comics.findIndex(c => c.id === item?.id);
    if (idx < 0) return null;
    return comics.slice(idx + 1).find(c => c.series === item?.series) ?? null;
  });
  const finished = $derived(!!item && !isPdf && count > 0 && (mode === 'double' ? page + 2 >= count : page >= count - 1));
  const pageLabel = $derived.by(() => {
    if (!item || isPdf) return '';
    if (mode === 'scroll') return `共 ${count} 页`;
    if (mode === 'double') return `${page + 1}-${Math.min(page + 2, count)} / ${count}`;
    return `${page + 1} / ${count}`;
  });

  // 打开 / 切换漫画
  $effect(() => {
    const cid = ui.comicId;
    if (cid == null) return;
    void loadComic(cid);
    return () => {
      saveNow();
    };
  });

  // 预加载前后几页
  $effect(() => {
    const it = item;
    const p = page;
    if (!it || it.format === 'pdf') return;
    for (let k = p - 2; k <= p + 3; k++) {
      if (k >= 0 && k < it.pages && k !== p) {
        const img = new Image();
        img.src = comicPage(it.id, k);
      }
    }
  });

  // 滚轮翻页 / Ctrl+滚轮缩放；长条模式下跟踪当前页
  $effect(() => {
    const el = pagesEl;
    if (!el) return;
    const wheel = (e: WheelEvent) => onWheel(e);
    const scroll = () => onPagesScroll();
    el.addEventListener('wheel', wheel, { passive: false });
    el.addEventListener('scroll', scroll, { passive: true });
    return () => {
      el.removeEventListener('wheel', wheel);
      el.removeEventListener('scroll', scroll);
    };
  });

  // 读完一本：提示下一卷
  $effect(() => {
    if (finished && !endNotified) {
      endNotified = true;
      toast(nextItem ? `读完了，可以接着看 ${nextItem.title}` : '已经读到最后一页');
    }
  });

  async function loadComic(cid: number) {
    loadedId = cid;
    endNotified = false;
    item = null;
    err = '';
    loading = true;
    aiOpen = false;
    zoom = 1;
    try {
      const list = await invoke<ComicItem[]>('list_comics');
      if (loadedId !== cid) return;
      comics = list;
      const it = list.find(x => x.id === cid);
      if (!it) {
        err = '找不到这本漫画';
        loading = false;
        return;
      }
      item = it;
      page = it.pages > 0 ? clamp(it.page, 0, it.pages - 1) : 0;
      loading = false;
    } catch (x) {
      if (loadedId === cid) {
        err = inTauri ? String(x) : '漫画阅读器需要在桌面程序中打开';
        loading = false;
      }
    }
  }

  function saveNow() {
    if (!loadedId || !item || item.format === 'pdf' || !inTauri) return;
    clearTimeout(saveTimer);
    invoke('save_comic_progress', { id: loadedId, page }).catch(() => {});
  }

  function saveSoon() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(saveNow, 300);
  }

  function setPage(k: number) {
    if (!item || page === k) return;
    page = k;
    saveSoon();
  }

  function scrollToPage(k: number) {
    const el = pagesEl?.querySelectorAll<HTMLElement>('.page')[k];
    el?.scrollIntoView({ block: 'start' });
  }

  /** 翻页：d 是阅读顺序上的方向，+1 为下一页 */
  function turn(d: number) {
    if (!item || isPdf) return;
    const step = mode === 'double' ? 2 : 1;
    let k = clamp(page + d * step, 0, Math.max(0, count - 1));
    if (mode === 'double') k -= k % 2;
    if (mode === 'scroll') {
      if (d > 0 && page >= count - 1) return;
      if (d < 0 && page <= 0) return;
      scrollToPage(k);
    }
    setPage(k);
  }

  function setMode(m: Mode) {
    if (m === 'double') page -= page % 2;
    mode = m;
    void tick().then(() => {
      if (m === 'scroll') scrollToPage(page);
    });
  }

  function toggleFull() {
    try {
      if (document.fullscreenElement) void document.exitFullscreen();
      else void document.documentElement.requestFullscreen().catch(() => toast('无法切换全屏'));
    } catch {
      toast('无法切换全屏');
    }
  }

  function toggleTr() {
    trMode = trMode === 'orig' ? 'tr' : 'orig';
  }

  function close() {
    saveNow();
    ui.comicId = null;
  }

  function gotoNext() {
    if (!nextItem) return;
    saveNow();
    ui.comicId = nextItem.id;
  }

  // 点击页面：左右半边翻页（日漫模式方向相反）。双击放大，所以单击稍等一下再翻
  function onPageClick(e: MouseEvent) {
    if (mode === 'scroll' || zoom > 1) return;
    const el = e.currentTarget as HTMLElement;
    const r = el.getBoundingClientRect();
    const right = e.clientX > r.left + r.width / 2;
    clearTimeout(clickTimer);
    clickTimer = setTimeout(() => {
      const forward = rtl ? !right : right;
      turn(forward ? 1 : -1);
    }, 230);
  }

  function onPageDbl() {
    clearTimeout(clickTimer);
    zoom = zoom > 1 ? 1 : 2;
  }

  function onWheel(e: WheelEvent) {
    if (e.ctrlKey) {
      e.preventDefault();
      zoom = clamp(Math.round(zoom * (e.deltaY < 0 ? 1.1 : 0.9) * 100) / 100, 0.5, 3);
      return;
    }
    if (mode === 'scroll' || zoom > 1) return;
    e.preventDefault();
    const now = Date.now();
    if (now - lastWheel < 300 || Math.abs(e.deltaY) < 4) return;
    lastWheel = now;
    turn(e.deltaY > 0 ? 1 : -1);
  }

  /** 长条模式：滚动时更新当前页 */
  function onPagesScroll() {
    if (mode !== 'scroll' || !pagesEl) return;
    const els = pagesEl.querySelectorAll<HTMLElement>('.page');
    const top = pagesEl.scrollTop + 60;
    let idx = 0;
    els.forEach((el, i) => {
      if (el.offsetTop <= top) idx = i;
    });
    setPage(idx);
  }

  function onKey(e: KeyboardEvent) {
    if (ui.comicId == null) return;
    if (typing(e)) {
      if (e.key === 'Escape') (e.target as HTMLElement).blur();
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      if (aiOpen) aiOpen = false;
      else close();
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      if (e.key === '0') {
        e.preventDefault();
        zoom = 1;
      } else if (e.key === '=' || e.key === '+') {
        e.preventDefault();
        zoom = clamp(zoom + 0.25, 0.5, 3);
      } else if (e.key === '-') {
        e.preventDefault();
        zoom = clamp(zoom - 0.25, 0.5, 3);
      }
      return;
    }
    if (isPdf) {
      if (e.key.toLowerCase() === 'f') toggleFull();
      return;
    }
    // 日漫模式：← 是下一页
    const fwd = rtl ? -1 : 1;
    switch (e.key) {
      case 'ArrowLeft':
        e.preventDefault();
        turn(-fwd);
        break;
      case 'ArrowRight':
        e.preventDefault();
        turn(fwd);
        break;
      case ' ':
      case 'PageDown':
        e.preventDefault();
        turn(1);
        break;
      case 'PageUp':
        e.preventDefault();
        turn(-1);
        break;
      case 'Home':
        e.preventDefault();
        setPage(0);
        if (mode === 'scroll') pagesEl?.scrollTo({ top: 0 });
        break;
      case 'End':
        e.preventDefault();
        setPage(Math.max(0, count - 1));
        if (mode === 'scroll') pagesEl?.scrollTo({ top: pagesEl.scrollHeight });
        break;
      case '1':
        setMode('single');
        break;
      case '2':
        setMode('double');
        break;
      case '3':
        setMode('scroll');
        break;
      case 't':
      case 'T':
        toggleTr();
        break;
      case 'f':
      case 'F':
        toggleFull();
        break;
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="reader open">
  <div class="rtop">
    <button type="button" class="rbtn" onclick={close}>← 书架</button>
    <b>{item?.title ?? '漫画'}</b>
    {#if item?.series}<span style="color:#999">{item.series}</span>{/if}
    <span class="sp"></span>
    {#if item && !isPdf}
      <button type="button" class="rbtn" onclick={() => turn(rtl ? 1 : -1)}>上一页</button>
      <span style="font-variant-numeric:tabular-nums;min-width:5em;text-align:center">{pageLabel}</span>
      <button type="button" class="rbtn" onclick={() => turn(rtl ? -1 : 1)}>下一页</button>
      <button type="button" class="rbtn" class:on={mode === 'single'} onclick={() => setMode('single')}>单页</button>
      <button type="button" class="rbtn" class:on={mode === 'double'} onclick={() => setMode('double')}>双页</button>
      <button type="button" class="rbtn" class:on={mode === 'scroll'} onclick={() => setMode('scroll')}>长条</button>
      <button type="button" class="rbtn" class:on={rtl} onclick={() => (rtl = !rtl)} title="日漫从右往左翻页">右→左</button>
      <button type="button" class="rbtn" class:on={trMode !== 'orig'} onclick={toggleTr} title="T">{trMode === 'orig' ? '原文' : '译文'}</button>
      <button type="button" class="rbtn" class:on={aiOpen} onclick={() => (aiOpen = !aiOpen)}>AI 翻译</button>
    {/if}
    {#if finished}
      {#if nextItem}
        <button type="button" class="rbtn pri" onclick={gotoNext}>下一卷：{nextItem.title} →</button>
      {:else}
        <span style="color:#7be0a3">已读完</span>
      {/if}
    {/if}
    <button type="button" class="rbtn" onclick={toggleFull} title="F">全屏</button>
  </div>

  <div class="rbody" class:nop={!aiOpen || isPdf}>
    {#if loading}
      <p class="msg">正在打开…</p>
    {:else if err}
      <p class="msg">{err}</p>
    {:else if item && isPdf}
      <iframe class="pdf" title={item.title} src={fileUrl(item.path)}></iframe>
    {:else if item}
      <div
        class="pages"
        class:rtl
        class:double={mode === 'double'}
        class:scroll={mode === 'scroll'}
        style="--z:{zoom}"
        bind:this={pagesEl}
      >
        {#each pageIdx as i (i)}
          <button
            type="button"
            class="page real"
            class:cur={i === page}
            class:cur2={mode === 'double' && i === page + 1}
            onclick={onPageClick}
            ondblclick={onPageDbl}
            aria-label="第 {i + 1} 页"
          >
            {#if mode === 'scroll' || Math.abs(i - page) <= 3}
              <img src={comicPage(item.id, i)} alt="第 {i + 1} 页" draggable="false" loading={mode === 'scroll' ? 'lazy' : 'eager'} />
            {/if}
            {#if trMode !== 'orig' || aiOpen}
              {#if Math.abs(i - page) <= 3 || mode === 'scroll'}
                <PageOverlay comicId={item.id} page={i} mode={trMode} />
              {/if}
            {/if}
            <span class="pgn">{i + 1}</span>
          </button>
        {/each}
      </div>
      {#if aiOpen}
        <TranslatePanel comicId={item.id} {page} pages={count} mode={trMode} onmode={m => (trMode = m)} />
      {/if}
    {/if}
  </div>
</div>

<style>
  .msg {
    color: #999;
    padding: 30px;
    margin: 0;
  }
  .pdf {
    grid-column: 1 / -1;
    width: 100%;
    height: 100%;
    min-height: 0;
    border: 0;
    background: #111;
  }
  /* 页面是真实图片：不用漫画样张的固定比例和底色 */
  .page.real {
    aspect-ratio: auto;
    height: auto;
    padding: 0;
    margin: 0;
    background: transparent;
    box-shadow: 0 10px 40px #000a;
    border: 0;
    font: inherit;
    color: inherit;
    cursor: pointer;
    display: block;
    flex: none;
  }
  .page.real img {
    display: block;
    height: calc((100vh - 100px) * var(--z, 1));
    width: auto;
    max-width: none;
    user-select: none;
  }
  .pages.double .page.real {
    max-width: 48%;
  }
  .pages.double .page.real img {
    max-width: 100%;
  }
  .pages.scroll .page.real {
    width: calc(min(680px, 100%) * var(--z, 1));
  }
  .pages.scroll .page.real img {
    width: 100%;
    height: auto;
  }
  .pages.rtl {
    flex-direction: row-reverse;
  }
  .pages.scroll {
    flex-direction: column;
  }
  .page.real .pgn {
    color: #ddd;
    background: #0008;
    border-radius: 4px;
    padding: 0 4px;
  }
</style>
