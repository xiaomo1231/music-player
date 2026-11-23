<script lang="ts">
  import { untrack } from 'svelte';
  import { invoke, inTauri, fileUrl, comicPage, setComicVer, type ComicItem } from '../lib/api';
  import { em } from '../lib/em.svelte';
  import { comicView, applyRuleRead, type ComicView } from '../lib/emcomic';
  import ComicExportModal from '../components/em/ComicExportModal.svelte';
  import { ui, toast } from '../lib/ui.svelte';
  import { typing, clamp } from '../lib/util';
  import {
    COMIC_FLOWS,
    COMIC_FITS,
    PAGE_ANIMS,
    COMIC_KEYS,
    COMIC_MOUSE,
    comicFor,
    comicGlobal,
    saveComicBook,
    saveComicGlobal,
    makeSpreads,
    spreadOf,
    COMIC_DEFAULTS,
    type ComicFlow,
    type ComicPrefs,
  } from '../lib/readerprefs';
  import TranslatePanel from '../components/mangatr/TranslatePanel.svelte';
  import PagedView from '../components/comic/PagedView.svelte';
  import ScrollView from '../components/comic/ScrollView.svelte';
  import ProgressBar from '../components/comic/ProgressBar.svelte';
  import { FLOW_ICON } from '../components/comic/icons';

  type Tr = 'orig' | 'tr' | 'both';

  let comics = $state.raw<ComicItem[]>([]);
  let item = $state.raw<ComicItem | null>(null);
  let err = $state('');
  let loading = $state(false);
  let page = $state(0);
  let prefs = $state<ComicPrefs>({ ...COMIC_DEFAULTS });
  let zoom = $state(1);
  let aiOpen = $state(false);
  let trMode = $state<Tr>('orig');
  let bars = $state(true);
  let pop = $state<'' | 'flow' | 'view' | 'rule'>('');
  let help = $state(false);
  let paged: PagedView | undefined = $state();
  /** 适用的漫画排版规则（.em） */
  let view = $state.raw<ComicView | null>(null);
  let exporting = $state(false);
  let scroller: ScrollView | undefined = $state();

  /** 当前加载的是哪一本（非响应式，给保存进度用） */
  let loadedId = 0;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let endNotified = false;
  /** 全局默认值（在阅读器里改“显示”里的选项时一起更新） */
  let gprefs: ComicPrefs = { ...COMIC_DEFAULTS };

  // ---------- 页面尺寸：图片加载后记下来，没加载的按已知页面的中位数估计 ----------
  let ratios = new Float64Array(0);
  let natH = new Float64Array(0);
  let natW = new Float64Array(0);
  let defRatio = 1.42;
  let dimsVer = $state(0);
  let bumpRaf = 0;
  const preload = new Map<number, HTMLImageElement>();

  function ondims(i: number, w: number, h: number) {
    if (i < 0 || i >= ratios.length || !w || !h) return;
    const r = h / w;
    if (Math.abs(ratios[i] - r) < 1e-4 && natH[i] === h) return;
    ratios[i] = r;
    natH[i] = h;
    natW[i] = w;
    if (!bumpRaf) {
      bumpRaf = requestAnimationFrame(() => {
        bumpRaf = 0;
        const known = Array.from(ratios).filter(x => x > 0 && x >= 1);
        if (known.length) {
          known.sort((a, b) => a - b);
          defRatio = known[known.length >> 1];
        }
        dimsVer++;
      });
    }
  }
  const ratioOf = (i: number) => ratios[i] || defRatio;
  const natOf = (i: number) => natH[i] || 0;
  const natWOf = (i: number) => natW[i] || 0;

  const isPdf = $derived(item?.format === 'pdf');
  const count = $derived(item ? item.pages : 0);
  const isPaged = $derived(prefs.flow === 'h' || prefs.flow === 'v');
  const horizontal = $derived(prefs.flow === 'h' || prefs.flow === 'hscroll');
  const rtl = $derived(horizontal && prefs.dir === 'rtl');
  const spreads = $derived.by(() => {
    void dimsVer;
    return makeSpreads(count, ratios, isPaged && prefs.spread === 'double', prefs.coverAlone);
  });
  const curSpread = $derived(isPaged && spreads.length ? spreads[spreadOf(spreads, page)] : [page]);
  const nextItem = $derived.by<ComicItem | null>(() => {
    if (!item) return null;
    const idx = comics.findIndex(c => c.id === item?.id);
    if (idx < 0) return null;
    return comics.slice(idx + 1).find(c => c.series === item?.series) ?? null;
  });
  const finished = $derived(!!item && !isPdf && count > 0 && curSpread[curSpread.length - 1] >= count - 1);
  const pageLabel = $derived.by(() => {
    if (!item || isPdf || !count) return '';
    const a = curSpread[0] + 1;
    const b = curSpread[curSpread.length - 1] + 1;
    return a === b ? `${a} / ${count}` : `${a}-${b} / ${count}`;
  });
  const flowInfo = $derived(COMIC_FLOWS.find(f => f.id === prefs.flow) ?? COMIC_FLOWS[0]);
  const fits = $derived(isPaged ? COMIC_FITS : COMIC_FITS.filter(f => f.id !== 'screen'));
  const fitNow = $derived(!isPaged && prefs.fit === 'screen' ? (prefs.flow === 'vscroll' ? 'width' : 'height') : prefs.fit);

  // 打开 / 切换漫画
  $effect(() => {
    const cid = ui.comicId;
    if (cid == null) return;
    void loadComic(cid);
    return () => {
      saveNow();
    };
  });

  // 排版规则变了：重新打开这本（页数、页面都可能变）
  let emSeen = em.version;
  $effect(() => {
    const v = em.version;
    untrack(() => {
      if (v === emSeen) return;
      emSeen = v;
      const id = loadedId;
      if (!id || ui.comicId !== id || !item || item.format === 'pdf') return;
      const cur = view;
      comicView(id)
        .then(nv => {
          if (loadedId !== id) return;
          const same = (nv?.rule?.id ?? '') === (cur?.rule?.id ?? '') && (nv?.rule ? nv.ver : '') === (cur?.ver ?? '');
          if (same) return;
          saveNow();
          void loadComic(id);
        })
        .catch(() => {});
    });
  });

  // 预加载前后几页（翻页时不闪），顺便拿到页面尺寸
  $effect(() => {
    const it = item;
    const p = page;
    const dbl = isPaged && prefs.spread === 'double';
    if (!it || it.format === 'pdf') return;
    const lo = p - (dbl ? 4 : 2);
    const hi = p + (dbl ? 6 : 4);
    for (let k = Math.max(0, lo); k <= Math.min(it.pages - 1, hi); k++) {
      if (preload.has(k)) continue;
      const img = new Image();
      img.decoding = 'async';
      img.onload = () => {
        if (loadedId === it.id) ondims(k, img.naturalWidth, img.naturalHeight);
      };
      img.src = comicPage(it.id, k);
      preload.set(k, img);
    }
    for (const k of [...preload.keys()]) if (k < lo - 4 || k > hi + 4) preload.delete(k);
  });

  // 读完一本：提示下一卷
  $effect(() => {
    if (finished && !endNotified) {
      endNotified = true;
      toast(nextItem ? `读完了，可以接着看 ${nextItem.title}` : '已经读到最后一页');
    }
  });

  /** 读一页的尺寸（用来判断是不是条漫），最多等 1.5 秒 */
  function probe(id: number, k: number): Promise<[number, number] | null> {
    if (!inTauri) return Promise.resolve(null);
    return new Promise(res => {
      const img = new Image();
      const t = setTimeout(() => res(null), 1500);
      img.onload = () => {
        clearTimeout(t);
        res([img.naturalWidth, img.naturalHeight]);
      };
      img.onerror = () => {
        clearTimeout(t);
        res(null);
      };
      img.src = comicPage(id, k);
    });
  }

  async function loadComic(cid: number) {
    loadedId = cid;
    endNotified = false;
    item = null;
    err = '';
    loading = true;
    aiOpen = false;
    zoom = 1;
    pop = '';
    preload.clear();
    try {
      const [list, pf, g] = await Promise.all([invoke<ComicItem[]>('list_comics'), comicFor(cid), comicGlobal()]);
      if (loadedId !== cid) return;
      comics = list;
      let it = list.find(x => x.id === cid);
      if (!it) {
        err = '找不到这本漫画';
        loading = false;
        return;
      }
      // 漫画排版规则：页数按排过之后的算，没单独设置过的书用规则里的阅读排版
      let v: ComicView | null = null;
      if (it.format !== 'pdf') {
        try {
          v = await comicView(cid);
        } catch (x) {
          toast(`排版规则没能生效：${x}`);
        }
        if (loadedId !== cid) return;
      }
      view = v?.rule ? v : null;
      if (v?.rule) {
        setComicVer(cid, v.ver);
        it = { ...it, pages: v.pages };
      }
      gprefs = g;
      ratios = new Float64Array(it.pages);
      natH = new Float64Array(it.pages);
      natW = new Float64Array(it.pages);
      defRatio = 1.42;
      const p0 = it.pages > 0 ? clamp(it.page, 0, it.pages - 1) : 0;
      let next = { ...pf.prefs };
      const ruleRead = !pf.has && view?.read ? view.read : null;
      if (ruleRead) next = applyRuleRead(next, ruleRead);
      // 没单独设置过的书：页面特别长的当作条漫，用上下连续滚动（规则指定了翻页方式时不改）
      if (it.format !== 'pdf' && it.pages > 0) {
        const d = await probe(cid, p0);
        if (loadedId !== cid) return;
        if (d) {
          ondims(p0, d[0], d[1]);
          if (!pf.has && !ruleRead?.flow && d[1] / d[0] >= 2 && (next.flow === 'h' || next.flow === 'v')) {
            next.flow = 'vscroll';
            toast('页面是长条，已切换为上下连续滚动（可在顶栏“翻页方式”里改）');
          }
        }
      }
      prefs = next;
      item = it;
      page = p0;
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

  /** 跳到第 k 页（进度条、Home / End） */
  function seek(k: number) {
    k = clamp(k, 0, Math.max(0, count - 1));
    if (!isPaged) scroller?.goto(k);
    setPage(k);
  }

  /** 翻页：d 是阅读顺序上的方向，+1 为下一页 */
  function turn(d: number) {
    if (!item || isPdf) return;
    if (isPaged) {
      if (!paged?.turn(d)) toast(d > 0 ? '已经是最后一页' : '已经是第一页');
    } else if (prefs.flow === 'hscroll') scroller?.scrollView(d * 0.85);
    else scroller?.stepPage(d);
  }

  // ---------- 偏好 ----------

  function saveBook() {
    if (loadedId) void saveComicBook(loadedId, prefs);
  }

  function setFlow(f: ComicFlow) {
    if (prefs.flow === f) return;
    prefs.flow = f;
    zoom = 1;
    saveBook();
  }

  function cycleFlow() {
    const ids = COMIC_FLOWS.map(f => f.id);
    setFlow(ids[(ids.indexOf(prefs.flow) + 1) % ids.length]);
    toast(`翻页方式：${COMIC_FLOWS.find(f => f.id === prefs.flow)?.name}`);
  }

  function setSpread(s: 'single' | 'double') {
    if (!isPaged) setFlow('h');
    prefs.spread = s;
    saveBook();
  }

  function setDir(d: 'ltr' | 'rtl') {
    prefs.dir = d;
    saveBook();
  }

  /** 改“显示”里的选项：当前阅读器立即生效，同时作为以后的默认值 */
  function setG<K extends 'fit' | 'anim' | 'gap' | 'size' | 'coverAlone'>(k: K, v: ComicPrefs[K]) {
    prefs[k] = v;
    gprefs = { ...gprefs, [k]: v };
    void saveComicGlobal(gprefs);
  }

  function setZoom(z: number) {
    zoom = clamp(Math.round(z * 100) / 100, 0.3, 4);
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

  const num = (e: Event) => Number((e.currentTarget as HTMLInputElement).value);

  function onWinDown(e: PointerEvent) {
    if (!pop) return;
    const t = e.target as HTMLElement;
    if (!t.closest('.cr-pop') && !t.closest('.cr-popbtn')) pop = '';
  }

  // ---------- 键盘 ----------

  function onKey(e: KeyboardEvent) {
    if (ui.comicId == null || exporting) return;
    if (typing(e)) {
      const t = e.target as HTMLInputElement;
      if (e.key !== 'Escape') return;
      t.blur();
      // 滑块、勾选框上按 Esc：顺便关掉弹出的面板
      if (t.type !== 'range' && t.type !== 'checkbox') return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      if (help) help = false;
      else if (pop) pop = '';
      else if (aiOpen) aiOpen = false;
      else close();
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      if (e.key === '0') {
        e.preventDefault();
        zoom = 1;
      } else if (e.key === '=' || e.key === '+') {
        e.preventDefault();
        setZoom(zoom + 0.25);
      } else if (e.key === '-') {
        e.preventDefault();
        setZoom(zoom - 0.25);
      }
      return;
    }
    if (isPdf) {
      if (e.key.toLowerCase() === 'f') toggleFull();
      return;
    }
    if (!item) return;
    const k = e.key;
    // 横向时 ← / → 按画面方向；日漫 ← 是下一页
    const fwd = rtl ? -1 : 1;
    switch (k) {
      case 'ArrowLeft':
      case 'ArrowRight': {
        e.preventDefault();
        turn(k === 'ArrowRight' ? fwd : -fwd);
        break;
      }
      case 'ArrowUp':
      case 'ArrowDown': {
        e.preventDefault();
        const d = k === 'ArrowDown' ? 1 : -1;
        if (prefs.flow === 'vscroll') scroller?.scrollView(d * 0.15);
        else if (prefs.flow === 'hscroll') turn(d);
        else if (!paged?.scrollWithin(d)) turn(d);
        break;
      }
      case ' ':
      case 'PageDown':
      case 'PageUp': {
        e.preventDefault();
        const d = k === 'PageUp' || e.shiftKey ? -1 : 1;
        if (isPaged) turn(d);
        else scroller?.scrollView(d * 0.9);
        break;
      }
      case 'Home':
        e.preventDefault();
        seek(0);
        break;
      case 'End':
        e.preventDefault();
        seek(count - 1);
        break;
      case '1':
        setSpread('single');
        break;
      case '2':
        setSpread('double');
        break;
      case '3':
        setFlow('vscroll');
        break;
      case 'm':
      case 'M':
        cycleFlow();
        break;
      case 'r':
      case 'R':
        if (horizontal) {
          setDir(prefs.dir === 'rtl' ? 'ltr' : 'rtl');
          toast(prefs.dir === 'rtl' ? '从右到左阅读（日漫）' : '从左到右阅读');
        }
        break;
      case 'h':
      case 'H':
        bars = !bars;
        break;
      case 't':
      case 'T':
        toggleTr();
        break;
      case 'f':
      case 'F':
        toggleFull();
        break;
      case '?':
        help = !help;
        break;
    }
  }
</script>

<svelte:window onkeydown={onKey} onpointerdown={onWinDown} />

{#snippet ico(name: string)}
  <svg class="cr-ico" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">{@html FLOW_ICON[name] ?? ''}</svg>
{/snippet}

<div class="reader open cr">
  {#if bars}
    <div class="rtop cr-top">
      <button type="button" class="rbtn" onclick={close}>← 书架</button>
      <b class="cr-title">{item?.title ?? '漫画'}</b>
      {#if item?.series}<span class="cr-series">{item.series}</span>{/if}
      {#if view?.rule && item}
        <div class="cr-wrap">
          <button type="button" class="rbtn cr-popbtn cr-rule" class:on={pop === 'rule'} onclick={() => (pop = pop === 'rule' ? '' : 'rule')} title="已按规则「{view.rule.name}」排版">
            排版<small>▾</small>
          </button>
          {#if pop === 'rule'}
            <div class="cr-pop l">
              <h6>已按规则「{view.rule.name}」排版</h6>
              <p class="cr-rinfo">
                {#if view.marker}这本是按这个规则导出的，页面不再处理，只套用阅读排版。
                {:else if view.processed}共 {view.pages} 页{view.skipped ? `，跳过了 ${view.skipped} 页` : ''}。页面在阅读时实时处理，原文件不变。
                {:else}只套用阅读排版，页面不变。{/if}
                阅读排版只在这本书没单独设置过时生效。
              </p>
              {#if !view.marker}
                <button type="button" class="rbtn" onclick={() => ((pop = ''), (exporting = true))}>导出排版后的 CBZ…</button>
              {/if}
            </div>
          {/if}
        </div>
      {/if}
      <span class="sp"></span>
      {#if item && !isPdf}
        <div class="cr-wrap">
          <button type="button" class="rbtn cr-popbtn" class:on={pop === 'flow'} onclick={() => (pop = pop === 'flow' ? '' : 'flow')} title="翻页方式（M）">
            {@render ico(prefs.flow)}<span class="cr-flowname">{flowInfo.name}</span><small>▾</small>
          </button>
          {#if pop === 'flow'}
            <div class="cr-pop l">
              <h6>翻页方式</h6>
              {#each COMIC_FLOWS as f (f.id)}
                <button type="button" class="cr-opt" class:on={prefs.flow === f.id} onclick={() => setFlow(f.id)}>
                  {@render ico(f.id)}
                  <span><b>{f.name}</b><small>{f.desc}</small></span>
                </button>
              {/each}
              <h6>翻页动画<small>{isPaged ? '' : '（翻页模式下有效）'}</small></h6>
              <div class="cr-seg">
                {#each PAGE_ANIMS as a (a.id)}
                  <button type="button" class:on={prefs.anim === a.id} disabled={!isPaged} onclick={() => setG('anim', a.id)}>{a.name}</button>
                {/each}
              </div>
            </div>
          {/if}
        </div>
        <div class="cr-seg" title={isPaged ? '' : '单页 / 双页只在翻页模式下可用'}>
          <button type="button" class:on={isPaged && prefs.spread === 'single'} disabled={!isPaged} onclick={() => setSpread('single')} title="1">单页</button>
          <button type="button" class:on={isPaged && prefs.spread === 'double'} disabled={!isPaged} onclick={() => setSpread('double')} title="2">双页</button>
        </div>
        <div class="cr-seg" title={horizontal ? '阅读方向（R）' : '阅读方向只对横向阅读有效'}>
          <button type="button" class:on={horizontal && prefs.dir === 'ltr'} disabled={!horizontal} onclick={() => setDir('ltr')}><span class="cr-long">从左到右</span><span class="cr-short">左→右</span></button>
          <button type="button" class:on={horizontal && prefs.dir === 'rtl'} disabled={!horizontal} onclick={() => setDir('rtl')}><span class="cr-long">从右到左</span><span class="cr-short">右→左</span></button>
        </div>
        <div class="cr-wrap">
          <button type="button" class="rbtn cr-popbtn" class:on={pop === 'view'} onclick={() => (pop = pop === 'view' ? '' : 'view')}>
            显示 {Math.round(zoom * 100)}%<small>▾</small>
          </button>
          {#if pop === 'view'}
            <div class="cr-pop">
              <h6>页面大小</h6>
              <div class="cr-seg">
                {#each fits as f (f.id)}
                  <button type="button" class:on={fitNow === f.id} onclick={() => setG('fit', f.id)}>{f.name}</button>
                {/each}
              </div>
              <h6>缩放<small>Ctrl + 滚轮、双击</small></h6>
              <div class="cr-row">
                <button type="button" class="rbtn" onclick={() => setZoom(zoom - 0.1)}>−</button>
                <input type="range" min="0.3" max="4" step="0.05" value={zoom} oninput={e => setZoom(num(e))} aria-label="缩放" />
                <button type="button" class="rbtn" onclick={() => setZoom(zoom + 0.1)}>+</button>
                <button type="button" class="rbtn" onclick={() => (zoom = 1)}>还原</button>
              </div>
              {#if isPaged}
                <label class="cr-chk"><input type="checkbox" checked={prefs.coverAlone} onchange={e => setG('coverAlone', (e.currentTarget as HTMLInputElement).checked)} /> 双页时封面单独一页</label>
              {:else}
                {#if fitNow === 'width'}
                  <h6>页面宽度 {prefs.size}%</h6>
                  <input type="range" min="20" max="100" step="5" value={prefs.size} oninput={e => setG('size', num(e))} aria-label="页面宽度" />
                {/if}
                <h6>页间距 {prefs.gap}px</h6>
                <input type="range" min="0" max="60" step="2" value={prefs.gap} oninput={e => setG('gap', num(e))} aria-label="页间距" />
              {/if}
            </div>
          {/if}
        </div>
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
      <button type="button" class="rbtn" class:on={help} onclick={() => (help = !help)} title="快捷键（?）">?</button>
    </div>
  {/if}

  <div class="rbody cr-body" class:nop={!aiOpen || isPdf}>
    <div class="cr-stage">
      {#if loading}
        <p class="msg">正在打开…</p>
      {:else if err}
        <p class="msg">{err}</p>
      {:else if item && isPdf}
        <iframe class="pdf" title={item.title} src={fileUrl(item.path)}></iframe>
      {:else if item && count > 0}
        {#key prefs.flow}
          {#if isPaged}
            <PagedView
              bind:this={paged}
              id={item.id}
              {spreads}
              {page}
              axis={prefs.flow === 'h' ? 'x' : 'y'}
              {rtl}
              anim={prefs.anim}
              fit={prefs.fit}
              {zoom}
              {dimsVer}
              {ratioOf}
              {natOf}
              tr={trMode}
              {ondims}
              onpage={setPage}
              onzoom={setZoom}
              onmiddle={() => (bars = !bars)}
            />
          {:else}
            <ScrollView
              bind:this={scroller}
              id={item.id}
              {count}
              {page}
              axis={prefs.flow === 'vscroll' ? 'y' : 'x'}
              {rtl}
              fit={fitNow}
              size={prefs.size}
              gap={prefs.gap}
              {zoom}
              {dimsVer}
              {ratioOf}
              {natOf}
              natW={natWOf}
              tr={trMode}
              {ondims}
              onpage={setPage}
              onzoom={setZoom}
              onmiddle={() => (bars = !bars)}
            />
          {/if}
        {/key}
        {#if !bars}
          <span class="cr-mini">{pageLabel}</span>
        {/if}
      {:else if item}
        <p class="msg">这本漫画没有可显示的页面</p>
      {/if}

      {#if help}
        <div class="cr-help" role="dialog" aria-label="快捷键">
          <div class="cr-help-head"><b>漫画阅读器快捷键</b><button type="button" class="rbtn" onclick={() => (help = false)}>关闭</button></div>
          <div class="cr-kv">
            {#each COMIC_KEYS as [k, v] (k)}<kbd>{k}</kbd><span>{v}</span>{/each}
          </div>
          <h6>鼠标 / 触摸板</h6>
          <div class="cr-kv">
            {#each COMIC_MOUSE as [k, v] (k)}<kbd>{k}</kbd><span>{v}</span>{/each}
          </div>
        </div>
      {/if}
    </div>
    {#if aiOpen && item && !isPdf}
      <TranslatePanel comicId={item.id} {page} pages={count} mode={trMode} onmode={m => (trMode = m)} />
    {/if}
  </div>

  {#if bars && item && !isPdf && count > 0}
    <div class="cr-bot">
      <button type="button" class="rbtn" onclick={() => turn(rtl ? 1 : -1)} title={rtl ? '下一页' : '上一页'}>{rtl ? '下一页' : '上一页'}</button>
      <ProgressBar id={item.id} {count} {page} reverse={rtl} onseek={seek} />
      <button type="button" class="rbtn" onclick={() => turn(rtl ? -1 : 1)} title={rtl ? '上一页' : '下一页'}>{rtl ? '上一页' : '下一页'}</button>
      <span class="cr-num">{pageLabel}</span>
    </div>
  {/if}
</div>

{#if exporting && item}
  <ComicExportModal
    ids={[item.id]}
    title={item.title}
    ruleName={view?.rule?.name}
    onclose={() => (exporting = false)}
    ondone={(r, replace) => {
      if (replace && r.outputs.length && loadedId) void loadComic(loadedId);
    }}
  />
{/if}

<style>
  .cr-rule {
    color: #7be0a3;
  }
  .cr-rinfo {
    margin: 0 2px 4px;
    font-size: 12px;
    line-height: 1.6;
    color: #aaa;
  }
  .reader.cr {
    grid-template-rows: auto minmax(0, 1fr) auto;
  }
  .cr-top {
    grid-row: 1;
    position: relative;
    z-index: 20;
  }
  .cr-body {
    grid-row: 2;
  }
  .cr-bot {
    grid-row: 3;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 16px 8px;
    border-top: 1px solid #ffffff14;
    font-size: 12.5px;
  }
  .cr-num {
    font-variant-numeric: tabular-nums;
    min-width: 5.5em;
    text-align: right;
    color: #ccc;
  }
  .cr-title {
    max-width: 22em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cr-series {
    color: #999;
  }
  .cr-short {
    display: none;
  }
  .cr-top {
    gap: 6px;
  }
  @media (max-width: 1100px) {
    .cr-series {
      display: none;
    }
    .cr-title {
      max-width: 8em;
    }
  }
  @media (max-width: 1000px) {
    .cr-long {
      display: none;
    }
    .cr-short {
      display: inline;
    }
    .cr-top .rbtn {
      padding: 5px 8px;
    }
  }
  .cr-stage {
    position: relative;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .msg {
    color: #999;
    padding: 30px;
    margin: 0;
  }
  .pdf {
    width: 100%;
    height: 100%;
    min-height: 0;
    border: 0;
    background: #111;
  }
  .cr-ico {
    width: 17px;
    height: 17px;
    flex: none;
  }
  .cr-popbtn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .cr-popbtn small {
    opacity: 0.6;
    font-size: 10px;
  }
  .cr-wrap {
    position: relative;
  }
  .cr-pop {
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    width: 330px;
    max-height: calc(100vh - 80px);
    overflow: auto;
    background: #1b1b22;
    border: 1px solid #ffffff22;
    border-radius: 12px;
    padding: 10px;
    box-shadow: 0 14px 40px #000c;
    display: grid;
    gap: 6px;
    z-index: 30;
  }
  .cr-pop.l {
    left: 0;
    right: auto;
  }
  .cr-pop h6 {
    margin: 6px 2px 0;
    font-size: 12px;
    font-weight: 600;
    color: #bbb;
  }
  .cr-pop h6 small {
    margin-left: 6px;
    font-weight: 400;
    color: #888;
  }
  .cr-opt {
    display: flex;
    gap: 10px;
    align-items: center;
    text-align: left;
    background: none;
    border: 1px solid transparent;
    border-radius: 9px;
    padding: 7px 9px;
    color: #eee;
    cursor: pointer;
  }
  .cr-opt:hover {
    background: #ffffff10;
  }
  .cr-opt.on {
    background: #ffffff1c;
    border-color: #ffffff30;
  }
  .cr-opt .cr-ico {
    width: 24px;
    height: 24px;
  }
  .cr-opt b {
    display: block;
    font-size: 13px;
  }
  .cr-opt small {
    display: block;
    font-size: 11.5px;
    color: #999;
  }
  .cr-seg {
    display: inline-flex;
    background: #ffffff10;
    border: 1px solid #ffffff1c;
    border-radius: 9px;
    padding: 2px;
    gap: 2px;
    flex-wrap: wrap;
  }
  .cr-seg button {
    background: none;
    border: 0;
    border-radius: 7px;
    padding: 4px 9px;
    font-size: 12.5px;
    color: #eee;
    cursor: pointer;
  }
  .cr-seg button.on {
    background: #fff;
    color: #111;
  }
  .cr-seg button:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .cr-row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .cr-row input {
    flex: 1;
    min-width: 0;
  }
  .cr-pop input[type='range'] {
    accent-color: #ff6a55;
  }
  .cr-chk {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 12.5px;
    margin-top: 4px;
  }
  .cr-mini {
    position: absolute;
    right: 12px;
    bottom: 10px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    background: #000a;
    color: #ddd;
    border-radius: 6px;
    padding: 2px 8px;
    pointer-events: none;
  }
  .cr-help {
    position: absolute;
    top: 16px;
    right: 16px;
    width: min(460px, calc(100% - 32px));
    max-height: calc(100% - 32px);
    overflow: auto;
    background: #1b1b22f2;
    border: 1px solid #ffffff22;
    border-radius: 12px;
    padding: 14px 16px;
    box-shadow: 0 14px 40px #000c;
    z-index: 25;
    font-size: 12.5px;
  }
  .cr-help-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 10px;
  }
  .cr-help h6 {
    margin: 14px 0 8px;
    font-size: 12.5px;
  }
  .cr-kv {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 12px;
    align-items: center;
  }
  .cr-kv kbd {
    font: inherit;
    font-size: 11.5px;
    background: #ffffff14;
    border: 1px solid #ffffff26;
    border-radius: 6px;
    padding: 1px 7px;
    white-space: nowrap;
    justify-self: start;
  }
</style>
