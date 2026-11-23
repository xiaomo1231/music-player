<script lang="ts">
  import { tick, untrack } from 'svelte';
  import { invoke, inTauri, type Book, type BookItem } from '../lib/api';
  import { ui, toast } from '../lib/ui.svelte';
  import { typing, clamp } from '../lib/util';
  import {
    cps,
    FONTS,
    DEFAULT_PREFS,
    loadPrefs,
    savePrefs,
    segments,
    offsetIn,
    hitMatches,
    loadBookmarks,
    saveBookmarks,
    type AdHit,
    type AdAct,
    type AdEstimate,
    type Bookmark,
    type ReaderPrefs,
  } from '../lib/reader';
  import {
    NOVEL_DEFAULTS,
    NOVEL_MODES,
    NOVEL_KEYS,
    SPEED_MIN,
    SPEED_MAX,
    novelFor,
    novelGlobal,
    saveNovelBook,
    saveNovelGlobal,
    type NovelMode,
    type NovelPrefs,
  } from '../lib/readerprefs';
  import AdPanel from '../components/reader/AdPanel.svelte';
  import TocPanel from '../components/reader/TocPanel.svelte';
  import ModeMenu from '../components/reader/ModeMenu.svelte';
  import { FLOW_ICON } from '../components/comic/icons';

  type Sel = { x: number; y: number; ci: number; pi: number; s: number; e: number; text: string };

  let book = $state.raw<Book | null>(null);
  let loading = $state(false);
  let err = $state('');
  let ci = $state(0);
  /** 章内滚动比例（0-1），同时作为保存的进度 */
  let ratio = $state(0);
  let prefs = $state<ReaderPrefs>(loadPrefs());
  let tocOpen = $state(false);
  let adOpen = $state(false);
  let raw = $state(false);
  let scanned = $state(false);
  let hits = $state<AdHit[]>([]);
  let candidates = $state<AdHit[]>([]);
  let estimate = $state<AdEstimate | null>(null);
  let busy = $state('');
  let bookmarks = $state<Bookmark[]>([]);
  let bodyEl: HTMLDivElement | undefined = $state();
  let selPop = $state<Sel | null>(null);
  let flashPara = $state(-1);
  let ttsOn = $state(false);
  let ttsPara = $state(-1);
  let focusSearch = $state(0);
  /** 翻页方式等（全局默认 + 每本书记住的翻页方式） */
  let np = $state<NovelPrefs>({ ...NOVEL_DEFAULTS });
  let menu = $state<{ x: number; y: number } | null>(null);
  let bars = $state(true);
  let help = $state(false);
  // 左右翻页：正文按屏分栏，translateX 翻页
  let pview: HTMLDivElement | undefined = $state();
  let pcols: HTMLDivElement | undefined = $state();
  let PW = $state(0);
  let PH = $state(0);
  /** 本章第几屏 / 共几屏 */
  let pi = $state(0);
  let pages = $state(1);
  let sliding = $state(false);
  let autoPaused = $state(false);
  let holding = false;

  /** 当前加载的是哪本书（非响应式，给保存进度用） */
  let loadedId = 0;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  let slideTimer: ReturnType<typeof setTimeout> | undefined;
  let flipAnim: Animation | null = null;
  let ngl: NovelPrefs = { ...NOVEL_DEFAULTS };
  let wheelAcc = 0;
  let wheelLast = 0;
  let wheelTurn = 0;
  /** 分栏之间的空隙（翻页时一次移动 PW + GAP） */
  const GAP = 48;

  const chapter = $derived(book && book.chapters[ci] ? book.chapters[ci] : null);
  const total = $derived(book ? book.chapters.length : 0);
  const pct = $derived(total ? ((ci + ratio) / total) * 100 : 0);
  const appliedCount = $derived(hits.filter(h => h.status === 'applied').length);
  /** 小说排版规则（.em）给的段首缩进、段间空行；没有规则时为 null，用默认样式 */
  const layout = $derived((book as (Book & { layout?: { indent: string; blank_lines: number } | null }) | null)?.layout ?? null);
  const indentEm = (s: string) => Array.from(s).reduce((n, c) => n + (c === '\u3000' ? 1 : 0.5), 0);
  const pStyle = $derived(
    `line-height:${prefs.lh};font-family:${FONTS[prefs.font]}` +
      (layout ? `;text-indent:${indentEm(layout.indent)}em;margin-bottom:${0.6 + layout.blank_lines * 1.2}em` : ''),
  );
  const paged = $derived(np.mode === 'page');
  const modeInfo = $derived(NOVEL_MODES.find(m => m.id === np.mode) ?? NOVEL_MODES[0]);

  /** 每段的渲染结果：已清理（隐藏）、待确认（高亮）、找相似的候选（高亮） */
  const rendered = $derived.by(() => {
    if (!chapter) return [];
    const inCh = hits.filter(h => h.chapter === ci && (h.status === 'applied' || h.status === 'pending'));
    const candCh = candidates.filter(h => h.chapter === ci);
    return chapter.paras.map((p, pi) => {
      const marks: { start: number; end: number; state: number }[] = [];
      for (const h of inCh) {
        if (h.para === pi && hitMatches(p, h)) marks.push({ start: h.start, end: h.end, state: h.status === 'applied' ? 2 : 1 });
      }
      for (const h of candCh) {
        if (h.para === pi && hitMatches(p, h)) marks.push({ start: h.start, end: h.end, state: 1 });
      }
      const r = segments(p, marks);
      return { pi, segs: r.segs, allGone: r.allGone };
    });
  });

  // 打开 / 切换书籍
  $effect(() => {
    const bid = ui.bookId;
    if (bid == null) return;
    void load(bid);
    return () => {
      saveNow();
      stopTts();
    };
  });

  // 绑定正文区域的事件（选中文字、滚动）
  $effect(() => {
    const el = bodyEl;
    if (!el) return;
    const up = (e: MouseEvent) => onSelect(e);
    const sc = () => {
      selPop = null;
      onScroll();
    };
    const wh = (e: WheelEvent) => onWheel(e);
    const down = () => (holding = true);
    const release = () => (holding = false);
    el.addEventListener('mouseup', up);
    el.addEventListener('scroll', sc, { passive: true });
    el.addEventListener('wheel', wh, { passive: false });
    el.addEventListener('pointerdown', down);
    window.addEventListener('pointerup', release);
    return () => {
      el.removeEventListener('mouseup', up);
      el.removeEventListener('scroll', sc);
      el.removeEventListener('wheel', wh);
      el.removeEventListener('pointerdown', down);
      window.removeEventListener('pointerup', release);
    };
  });

  // 左右翻页：字号、窗口大小、章节内容变化后重新分页，停在同样的位置（章内比例）
  $effect(() => {
    if (!paged) return;
    void PW;
    void PH;
    void prefs.fs;
    void prefs.lh;
    void prefs.font;
    void rendered;
    void raw;
    const r = untrack(() => ratio);
    void tick().then(() => {
      if (!measure()) return;
      setPi(Math.round(r * (pages - 1)), false);
    });
  });

  // 自动滚动：按速度匀速往下滚，滚到章末停一下接着下一章
  $effect(() => {
    if (np.mode !== 'auto' || autoPaused || !bodyEl || !book) return;
    let raf = 0;
    let last = performance.now();
    // 自己记小数位置：直接 scrollTop += 小数会被对齐到物理像素，速度不准
    let pos = bodyEl.scrollTop;
    let endAt = 0;
    const step = (now: number) => {
      const el = bodyEl;
      if (!el) return;
      const dt = Math.min(100, now - last);
      last = now;
      if (Math.abs(el.scrollTop - pos) > 2) pos = el.scrollTop;
      if (!holding && !selPop) {
        pos = Math.min(pos + (np.speed * dt) / 1000, el.scrollHeight - el.clientHeight);
        el.scrollTop = pos;
        if (el.scrollTop + el.clientHeight >= el.scrollHeight - 1) {
          if (!endAt) endAt = now;
          else if (now - endAt > 1500) {
            endAt = 0;
            if (ci + 1 < total) gotoChapter(ci + 1, 0);
            else {
              autoPaused = true;
              toast('已读完全书');
              return;
            }
          }
        } else endAt = 0;
      }
      raf = requestAnimationFrame(step);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  });

  // 外观偏好记在本机
  $effect(() => {
    savePrefs({ ...prefs });
  });

  async function load(bid: number) {
    loadedId = bid;
    book = null;
    err = '';
    loading = true;
    hits = [];
    candidates = [];
    estimate = null;
    scanned = false;
    adOpen = false;
    raw = false;
    try {
      const [b, items, marks, hs, pf, g] = await Promise.all([
        invoke<Book>('open_book', { id: bid }),
        invoke<BookItem[]>('list_books'),
        loadBookmarks(bid),
        invoke<AdHit[]>('adclean_list', { id: bid }),
        novelFor(bid),
        novelGlobal(),
      ]);
      if (loadedId !== bid) return;
      np = pf;
      ngl = g;
      autoPaused = false;
      const item = items.find(x => x.id === bid);
      book = b;
      ci = clamp(item?.chapter ?? 0, 0, Math.max(0, b.chapters.length - 1));
      ratio = item?.offset ?? 0;
      bookmarks = marks;
      hits = hs;
      loading = false;
      await tick();
      applyScroll();
    } catch (x) {
      if (loadedId === bid) {
        err = inTauri ? String(x) : '阅读器需要在桌面程序中打开';
        loading = false;
      }
    }
  }

  function currentRatio(): number {
    if (paged) return pages > 1 ? pi / (pages - 1) : 0;
    const el = bodyEl;
    if (!el) return ratio;
    const max = el.scrollHeight - el.clientHeight;
    return max > 0 ? Math.min(1, el.scrollTop / max) : 0;
  }

  function applyScroll() {
    if (paged) {
      if (measure()) setPi(Math.round(ratio * (pages - 1)), false);
      return;
    }
    const el = bodyEl;
    if (!el) return;
    el.scrollTop = ratio * Math.max(0, el.scrollHeight - el.clientHeight);
  }

  // ---------- 左右翻页 ----------

  /** 量出本章分成几屏，返回是否量成了 */
  function measure(): boolean {
    if (!pcols || PW < 50) return false;
    pages = Math.max(1, Math.ceil((pcols.scrollWidth + GAP - 4) / (PW + GAP)));
    return true;
  }

  function setPi(k: number, anim: boolean) {
    k = clamp(k, 0, Math.max(0, pages - 1));
    clearTimeout(slideTimer);
    sliding = anim;
    if (anim) slideTimer = setTimeout(() => (sliding = false), 400);
    pi = k;
    ratio = pages > 1 ? k / (pages - 1) : 0;
    if (pview) pview.scrollLeft = 0;
  }

  function saveSoon() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(saveNow, 1200);
  }

  /** 左右翻页：d = +1 下一屏 / -1 上一屏，到章首章尾时换章 */
  function turnPage(d: number) {
    if (!book || !paged) return;
    if (flipAnim) flipAnim.finish();
    const k = pi + d;
    if (k < 0) {
      if (ci > 0) gotoChapter(ci - 1, 1);
      else toast('已经是第一页');
      return;
    }
    if (k >= pages) {
      if (ci + 1 < total) gotoChapter(ci + 1, 0);
      else toast('已经是最后一页');
      return;
    }
    if (np.anim === 'flip') flipTo(k, d);
    else setPi(k, np.anim === 'slide');
    saveSoon();
  }

  /** 仿真翻页：复制一份当前排版，绕左边“书脊”转过去（往回翻时转回来） */
  function flipTo(k: number, d: number) {
    const src = pcols;
    const host = pview;
    if (!src || !host) return setPi(k, false);
    const wrap = document.createElement('div');
    const bg = getComputedStyle(host.closest('.mp-reader') ?? host).backgroundColor;
    wrap.style.cssText = `position:absolute;inset:0;overflow:hidden;background:${bg};backface-visibility:hidden;transform-origin:left center;z-index:2;box-shadow:0 0 30px #0004;pointer-events:none`;
    const clone = src.cloneNode(true) as HTMLElement;
    clone.style.transition = 'none';
    clone.style.transform = `translateX(${-(d > 0 ? pi : k) * (PW + GAP)}px)`;
    wrap.appendChild(clone);
    host.appendChild(wrap);
    const out = [
      { transform: 'perspective(1800px) rotateY(0deg)', filter: 'brightness(1)' },
      { transform: 'perspective(1800px) rotateY(-90deg)', filter: 'brightness(.6)' },
    ];
    if (d > 0) setPi(k, false);
    const a = wrap.animate(d > 0 ? out : [...out].reverse(), { duration: 420, easing: d > 0 ? 'ease-in' : 'ease-out' });
    flipAnim = a;
    const done = () => {
      wrap.remove();
      if (flipAnim === a) flipAnim = null;
      if (d < 0) setPi(k, false);
    };
    a.onfinish = done;
    a.oncancel = done;
  }

  /** 让第 p 段出现在屏幕上（左右翻页时翻到它所在的那一屏） */
  function showPara(c: number, p: number, smooth = false) {
    const el = document.getElementById(`p-${c}-${p}`);
    if (!el) return;
    if (paged) {
      if (measure()) setPi(Math.floor((el.offsetLeft + 2) / (PW + GAP)), false);
    } else el.scrollIntoView({ block: 'center', behavior: smooth ? 'smooth' : 'auto' });
  }

  function onPageClick(e: MouseEvent) {
    if (!paged || !pview) return;
    if ((e.target as HTMLElement).closest('button')) return;
    const sel = window.getSelection();
    if (sel && !sel.isCollapsed) return;
    const r = pview.getBoundingClientRect();
    const f = (e.clientX - r.left) / r.width;
    if (f < 1 / 3) turnPage(-1);
    else if (f > 2 / 3) turnPage(1);
    else bars = !bars;
  }

  function onWheel(e: WheelEvent) {
    if (!paged || e.ctrlKey) return;
    e.preventDefault();
    const now = performance.now();
    if (now - wheelLast > 250) wheelAcc = 0;
    wheelLast = now;
    const d = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
    wheelAcc += d;
    if (Math.abs(wheelAcc) < 30 || now - wheelTurn < 350) return;
    wheelTurn = now;
    const s = Math.sign(wheelAcc);
    wheelAcc = 0;
    turnPage(s);
  }

  // ---------- 翻页方式 ----------

  async function setMode(m: NovelMode) {
    if (np.mode === m) return;
    const r = currentRatio();
    if (flipAnim) flipAnim.finish();
    np.mode = m;
    autoPaused = false;
    if (loadedId) void saveNovelBook(loadedId, np);
    ratio = r;
    await tick();
    applyScroll();
  }

  function cycleMode() {
    const ids = NOVEL_MODES.map(m => m.id);
    void setMode(ids[(ids.indexOf(np.mode) + 1) % ids.length]).then(() =>
      toast(`翻页方式：${NOVEL_MODES.find(m => m.id === np.mode)?.name}`),
    );
  }

  function setGlobal<K extends 'anim' | 'speed'>(k: K, v: NovelPrefs[K]) {
    np[k] = v;
    ngl = { ...ngl, [k]: v };
    void saveNovelGlobal(ngl);
  }

  function setSpeed(v: number) {
    setGlobal('speed', clamp(Math.round(v), SPEED_MIN, SPEED_MAX));
  }

  function openMenu(e: MouseEvent) {
    if (menu) {
      menu = null;
      return;
    }
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu = { x: Math.min(r.left, window.innerWidth - 310), y: r.bottom + 6 };
  }

  /** 上下滚动：按屏滚动 */
  function scrollScreen(d: number) {
    const el = bodyEl;
    if (!el) return;
    el.scrollBy({ top: d * el.clientHeight * 0.9, behavior: 'smooth' });
  }

  function saveNow() {
    if (!loadedId || !book || !inTauri) return;
    const r = bodyEl ? currentRatio() : ratio;
    invoke('save_book_progress', { id: loadedId, chapter: ci, offset: r, chapters: book.chapters.length }).catch(() => {});
  }

  function onScroll() {
    if (paged) {
      // 分栏模式下正文不滚动（选中文字等可能把它带偏），复位
      if (bodyEl && bodyEl.scrollTop) bodyEl.scrollTop = 0;
      return;
    }
    ratio = currentRatio();
    clearTimeout(saveTimer);
    saveTimer = setTimeout(saveNow, 1200);
  }

  function gotoChapter(n: number, at = 0, keepTts = false) {
    if (!book) return;
    const k = clamp(n, 0, book.chapters.length - 1);
    if (!keepTts) stopTts();
    ci = k;
    ratio = at;
    pi = 0;
    flashPara = -1;
    if (bodyEl) bodyEl.scrollTop = 0;
    tick().then(() => {
      applyScroll();
      saveNow();
    });
  }

  function prevChapter() {
    if (ci <= 0) return toast('已经是第一章');
    gotoChapter(ci - 1);
  }

  function nextChapter() {
    if (ci + 1 >= total) return toast('已经是最后一章');
    gotoChapter(ci + 1);
  }

  async function jumpTo(c: number, p: number) {
    if (c !== ci) gotoChapter(c);
    await tick();
    showPara(c, p);
    flashPara = p;
    setTimeout(() => {
      if (flashPara === p) flashPara = -1;
    }, 1500);
    ratio = currentRatio();
  }

  function close() {
    saveNow();
    stopTts();
    ui.bookId = null;
  }

  function setFs(d: number) {
    prefs.fs = clamp(prefs.fs + d, 14, 32);
  }

  function setTheme(t: ReaderPrefs['theme']) {
    prefs.theme = t;
  }

  function toggleFull() {
    try {
      if (document.fullscreenElement) void document.exitFullscreen();
      else void document.documentElement.requestFullscreen().catch(() => toast('无法切换全屏'));
    } catch {
      toast('无法切换全屏');
    }
  }

  // ---------- 书签 ----------

  /** 当前屏幕上第一段文字，用作书签的名字 */
  /** 屏幕上第一段（上下滚动看纵向，左右翻页看当前这一屏） */
  function firstVisible(): HTMLElement | null {
    const el = bodyEl;
    if (!el) return null;
    const v = (paged && pview ? pview : el).getBoundingClientRect();
    for (const p of el.querySelectorAll<HTMLElement>('p[data-p]')) {
      const r = p.getBoundingClientRect();
      if (r.bottom > v.top + 4 && r.top < v.bottom && r.right > v.left + 4 && r.left < v.right - 4) return p;
    }
    return null;
  }

  function visibleLabel(): string {
    const p = firstVisible();
    if (p) return cps((p.textContent ?? '').trim()).slice(0, 18).join('');
    return chapter?.title ?? '';
  }

  async function toggleBookmark() {
    if (!loadedId || !book) return;
    const r = currentRatio();
    const idx = bookmarks.findIndex(b => b.chapter === ci && Math.abs(b.offset - r) < 0.02);
    let list: Bookmark[];
    if (idx >= 0) {
      list = bookmarks.filter((_, i) => i !== idx);
      toast('已删除书签');
    } else {
      list = [...bookmarks, { chapter: ci, offset: r, label: visibleLabel(), at: Math.floor(Date.now() / 1000) }];
      toast('已加入书签（B 键可切换）');
    }
    bookmarks = list;
    await saveBookmarks(loadedId, list).catch(() => toast('书签保存失败'));
  }

  async function removeBookmark(i: number) {
    if (!loadedId) return;
    const list = bookmarks.filter((_, k) => k !== i);
    bookmarks = list;
    await saveBookmarks(loadedId, list).catch(() => toast('书签保存失败'));
  }

  // ---------- 朗读 ----------

  function stopTts() {
    ttsOn = false;
    ttsPara = -1;
    if (typeof window !== 'undefined' && 'speechSynthesis' in window) window.speechSynthesis.cancel();
  }

  function firstVisiblePara(): number {
    const p = firstVisible();
    return p ? Number(p.dataset.p) : 0;
  }

  function toggleTts() {
    if (ttsOn) return stopTts();
    if (!('speechSynthesis' in window)) return toast('当前环境不支持朗读');
    ttsOn = true;
    speakFrom(firstVisiblePara());
  }

  /** 从第 pi 段开始朗读，读完一段接下一段，读完一章翻到下一章 */
  function speakFrom(pi: number) {
    if (!ttsOn || !chapter) return;
    if (pi >= chapter.paras.length) {
      if (ci + 1 < total) {
        gotoChapter(ci + 1, 0, true);
        void tick().then(() => speakFrom(0));
      } else {
        stopTts();
        toast('已读完全书');
      }
      return;
    }
    const item = rendered[pi];
    const text = item ? item.segs.filter(s => s.s !== 2).map(s => s.t).join('') : chapter.paras[pi];
    ttsPara = pi;
    showPara(ci, pi, true);
    if (!text.trim()) return speakFrom(pi + 1);
    const u = new SpeechSynthesisUtterance(text);
    u.lang = 'zh-CN';
    u.rate = 1;
    u.onend = () => {
      if (ttsOn && ttsPara === pi) speakFrom(pi + 1);
    };
    window.speechSynthesis.speak(u);
  }

  // ---------- 去广告：标记、清理 ----------

  /** 找出选区所在的段落；跨段的选区不处理 */
  function paraOf(n: Node): HTMLElement | null {
    const el = n instanceof Element ? n : n.parentElement;
    return (el?.closest('p[data-p]') as HTMLElement | null) ?? null;
  }

  function onSelect(e: MouseEvent) {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || !sel.rangeCount) {
      selPop = null;
      return;
    }
    const r = sel.getRangeAt(0);
    const a = paraOf(r.startContainer);
    const b = paraOf(r.endContainer);
    if (!a || !b) {
      selPop = null;
      return;
    }
    if (a !== b) {
      selPop = null;
      toast('请在同一段里选择要标记的文字');
      return;
    }
    const s = offsetIn(a, r.startContainer, r.startOffset);
    const en = offsetIn(a, r.endContainer, r.endOffset);
    const text = cps(a.textContent ?? '').slice(s, en).join('');
    if (en <= s || cps(text.trim()).length < 2) {
      selPop = null;
      return;
    }
    selPop = { x: e.clientX, y: e.clientY, ci, pi: Number(a.dataset.p), s, e: en, text };
  }

  async function refreshHits() {
    if (!loadedId) return;
    hits = await invoke<AdHit[]>('adclean_list', { id: loadedId });
  }

  async function markSelection(similar: boolean) {
    const sp = selPop;
    selPop = null;
    window.getSelection()?.removeAllRanges();
    if (!sp || !loadedId) return;
    const id = loadedId;
    const item: AdHit = {
      chapter: sp.ci,
      para: sp.pi,
      start: sp.s,
      end: sp.e,
      text: sp.text,
      rule: '手动标记',
      rule_id: null,
      confidence: 'hi',
      status: '',
    };
    try {
      await invoke('adclean_apply', { id, items: [item] });
      await refreshHits();
      adOpen = true;
      if (similar) {
        busy = '正在全书查找相似内容…';
        const list = await invoke<AdHit[]>('adclean_find_similar', { id, sample: sp.text });
        candidates = list.filter(h => h.status !== 'applied');
        toast(candidates.length ? `找到 ${candidates.length} 处相似内容，请确认后清理` : '全书没有找到相似内容，只清理了这一处');
      } else {
        toast('已标记为广告，阅读时隐藏');
      }
    } catch (x) {
      toast(String(x));
    } finally {
      busy = '';
    }
  }

  async function exportClean() {
    if (!loadedId || !book) return;
    const { save } = await import('@tauri-apps/plugin-dialog');
    const path = await save({ defaultPath: `${book.title}（净化版）.txt`, filters: [{ name: '文本', extensions: ['txt'] }] });
    if (!path) return;
    const n = await invoke<number>('adclean_export_clean', { id: loadedId, path });
    toast(`已导出干净的副本（清理了 ${n} 处），原文件未改动`);
  }

  async function onAct(a: AdAct) {
    if (!loadedId) return;
    const id = loadedId;
    try {
      switch (a.k) {
        case 'scan':
          busy = '正在扫描全书…';
          await invoke('adclean_scan', { id });
          scanned = true;
          await refreshHits();
          toast(`扫描完成：已清理 ${appliedCount} 处，待确认 ${hits.filter(h => h.status === 'pending').length} 处`);
          break;
        case 'estimate':
          estimate = await invoke<AdEstimate>('adclean_estimate', { id });
          break;
        case 'ai': {
          estimate = null;
          busy = 'AI 正在识别可疑内容…';
          const rules = await invoke<unknown[]>('adclean_ai_rules', { id, samples: null });
          await invoke('adclean_scan', { id });
          scanned = true;
          await refreshHits();
          toast(`AI 归纳出 ${rules.length} 条规则，已重新扫描`);
          break;
        }
        case 'confirm':
          await invoke('adclean_apply', { id, items: a.items });
          await refreshHits();
          break;
        case 'cancel':
          if (a.item.id == null) break;
          await invoke('adclean_revert', { id, cleanupIds: [a.item.id] });
          await refreshHits();
          break;
        case 'revertAll':
          await invoke('adclean_revert', { id, cleanupIds: [] });
          await refreshHits();
          toast('已全部撤销，恢复为原文');
          break;
        case 'jump':
          await jumpTo(a.hit.chapter, a.hit.para);
          break;
        case 'applyCandidates':
          await invoke('adclean_apply', { id, items: a.items });
          candidates = [];
          await refreshHits();
          toast(`已清理 ${a.items.length} 处`);
          break;
        case 'dismiss':
          candidates = [];
          estimate = null;
          break;
        case 'raw':
          raw = !raw;
          break;
        case 'export':
          await exportClean();
          break;
      }
    } catch (x) {
      toast(String(x));
    } finally {
      busy = '';
    }
  }

  // ---------- 键盘 ----------

  function onKey(e: KeyboardEvent) {
    if (ui.bookId == null) return;
    const inField = typing(e);
    if (inField) {
      if (e.key === 'Escape') (e.target as HTMLElement).blur();
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      if (help) help = false;
      else if (menu) menu = null;
      else if (selPop) selPop = null;
      else if (tocOpen || adOpen) {
        tocOpen = false;
        adOpen = false;
      } else close();
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      const k = e.key.toLowerCase();
      if (e.key === '=' || e.key === '+') {
        e.preventDefault();
        setFs(2);
      } else if (e.key === '-') {
        e.preventDefault();
        setFs(-2);
      } else if (e.key === '0') {
        e.preventDefault();
        prefs.fs = DEFAULT_PREFS.fs;
      } else if (k === 'f') {
        e.preventDefault();
        tocOpen = true;
        focusSearch++;
      }
      return;
    }
    const k = e.key;
    if (paged && ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', ' '].includes(k)) {
      e.preventDefault();
      const back = k === 'ArrowLeft' || k === 'ArrowUp' || k === 'PageUp' || (k === ' ' && e.shiftKey);
      turnPage(back ? -1 : 1);
      return;
    }
    switch (k) {
      case 'ArrowRight':
      case 'PageDown':
        e.preventDefault();
        nextChapter();
        break;
      case 'ArrowLeft':
      case 'PageUp':
        e.preventDefault();
        prevChapter();
        break;
      case ' ':
        e.preventDefault();
        if (np.mode === 'auto') {
          autoPaused = !autoPaused;
          toast(autoPaused ? '自动滚动已暂停（空格继续）' : '继续自动滚动');
        } else scrollScreen(e.shiftKey ? -1 : 1);
        break;
      case 'ArrowUp':
      case 'ArrowDown':
        if (np.mode === 'auto') {
          e.preventDefault();
          setSpeed(np.speed + (k === 'ArrowUp' ? 5 : -5));
        }
        break;
      case 'Home':
        e.preventDefault();
        if (paged) setPi(0, false);
        else bodyEl?.scrollTo({ top: 0 });
        break;
      case 'End':
        e.preventDefault();
        if (paged) setPi(pages - 1, false);
        else if (bodyEl) bodyEl.scrollTo({ top: bodyEl.scrollHeight });
        break;
      case 'm':
      case 'M':
        cycleMode();
        break;
      case 'h':
      case 'H':
        bars = !bars;
        break;
      case '?':
        help = !help;
        break;
      case 'f':
      case 'F':
        toggleFull();
        break;
      case 'b':
      case 'B':
        void toggleBookmark();
        break;
    }
  }
</script>

<svelte:window onkeydown={onKey} />

{#snippet content(ch: { title: string })}
  <h2>{ch.title}</h2>
  {#each rendered as r (r.pi)}
    <p
      data-p={r.pi}
      id="p-{ci}-{r.pi}"
      style={pStyle}
      class:adp={r.allGone}
      class:gone={r.allGone}
      class:tts={ttsPara === r.pi}
      class:flash={flashPara === r.pi}
    >{#each r.segs as sg, si (si)}{#if sg.s === 2}<span class="ad gone">{sg.t}</span>{:else if sg.s === 1}<span class="ad found">{sg.t}</span>{:else}{sg.t}{/if}{/each}</p>
  {/each}
  <div class="chapnav">
    <button type="button" onclick={prevChapter} disabled={ci <= 0}>← 上一章</button>
    <span>{ci + 1} / {total}</span>
    <button type="button" onclick={nextChapter} disabled={ci + 1 >= total}>下一章 →</button>
  </div>
{/snippet}


<div class="mp-over mp-reader open" class:green={prefs.theme === 'green'} class:night={prefs.theme === 'night'} class:adopen={adOpen} class:showraw={raw}>
  <div class="mp-rhead" class:nobar={!bars}>
    <button type="button" onclick={close}>← 书架</button>
    <b>{book?.title ?? '小说'}</b>
    <span>{chapter ? chapter.title : ''}{paged && book ? ` · ${pi + 1} / ${pages}` : ''}</span>
    <span style="flex:1"></span>
    <button type="button" class="nm-btn" class:on={!!menu} onclick={openMenu} title="翻页方式（M）">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">{@html FLOW_ICON[np.mode] ?? ''}</svg>
      {modeInfo.name} ▾
    </button>
    <button type="button" onclick={() => (tocOpen = !tocOpen)}>目录</button>
    <button type="button" onclick={() => setFs(-2)} title="Ctrl + 减">A−</button>
    <button type="button" onclick={() => setFs(2)} title="Ctrl + 加">A+</button>
    <button type="button" onclick={() => (prefs.lh = clamp(Math.round((prefs.lh - 0.2) * 10) / 10, 1.4, 3))}>行距−</button>
    <button type="button" onclick={() => (prefs.lh = clamp(Math.round((prefs.lh + 0.2) * 10) / 10, 1.4, 3))}>行距+</button>
    <select bind:value={prefs.font} aria-label="字体" style="background:transparent;color:inherit;border:1px solid currentColor;border-radius:8px;padding:2px 6px;font-size:12px">
      <option value="song">宋体</option>
      <option value="hei">黑体</option>
      <option value="kai">楷体</option>
    </select>
    <button type="button" onclick={() => setTheme('paper')}>纸张</button>
    <button type="button" onclick={() => setTheme('green')}>护眼</button>
    <button type="button" onclick={() => setTheme('night')}>夜间</button>
    <button type="button" onclick={toggleTts}>{ttsOn ? '停止朗读' : '朗读'}</button>
    <button type="button" onclick={toggleBookmark} title="B">书签</button>
    <button type="button" onclick={() => { tocOpen = true; focusSearch++; }} title="Ctrl + F">搜索</button>
    <button type="button" onclick={() => (adOpen = !adOpen)}>去广告{appliedCount ? ` · ${appliedCount}` : ''}</button>
    <button type="button" onclick={toggleFull} title="F">全屏</button>
    <button type="button" onclick={() => (help = !help)} title="快捷键（?）">?</button>
  </div>
  <div class="mp-rprog" class:nobar={!bars}><i style="width:{pct}%"></i></div>

  <div class="mp-rwrap">
    <div class="toc" class:open={tocOpen}>
      {#if book}
        <TocPanel
          chapters={book.chapters}
          current={ci}
          {bookmarks}
          {focusSearch}
          onchapter={c => gotoChapter(c)}
          onpara={(c, p) => jumpTo(c, p)}
          onbookmark={b => gotoChapter(b.chapter, b.offset)}
          onremove={i => removeBookmark(i)}
        />
      {/if}
    </div>

    <div class="mp-rbody" class:pmode={paged} style="--fs:{prefs.fs}px" bind:this={bodyEl}>
      {#if loading}
        <p class="hint">正在打开…</p>
      {:else if err}
        <p class="hint">{err}</p>
      {:else if chapter && paged}
        <!-- 左右翻页：整章排成等宽的栏，每栏正好一屏，移动整排来翻页 -->
        <div class="pview" bind:this={pview} bind:clientWidth={PW} bind:clientHeight={PH} onclick={onPageClick} role="presentation">
          <div
            class="pcols"
            class:slide={sliding}
            bind:this={pcols}
            style="column-width:{PW}px;column-gap:{GAP}px;height:{PH}px;transform:translateX({-pi * (PW + GAP)}px)"
          >
            {@render content(chapter)}
          </div>
        </div>
      {:else if chapter}
        {@render content(chapter)}
      {/if}
    </div>

    <div class="mp-rside">
      <AdPanel {hits} {candidates} {busy} {estimate} {raw} {scanned} act={onAct} />
    </div>
  </div>

  {#if menu}
    <ModeMenu
      mode={np.mode}
      anim={np.anim}
      speed={np.speed}
      x={menu.x}
      y={menu.y}
      onmode={m => void setMode(m)}
      onanim={a => setGlobal('anim', a)}
      onspeed={setSpeed}
      onclose={() => (menu = null)}
    />
  {/if}

  {#if np.mode === 'auto' && book}
    <div class="autobar">
      <button type="button" onclick={() => (autoPaused = !autoPaused)} title="空格">{autoPaused ? '▶ 继续' : '❚❚ 暂停'}</button>
      <button type="button" onclick={() => setSpeed(np.speed - 5)} title="↓">慢</button>
      <span>速度 {np.speed}</span>
      <button type="button" onclick={() => setSpeed(np.speed + 5)} title="↑">快</button>
    </div>
  {/if}

  {#if paged && book && !bars}
    <span class="pinfo">{pi + 1} / {pages}</span>
  {/if}

  {#if help}
    <div class="nhelp" role="dialog" aria-label="快捷键">
      <div class="nhelp-head"><b>小说阅读器快捷键</b><button type="button" onclick={() => (help = false)}>关闭</button></div>
      <div class="nkv">
        {#each NOVEL_KEYS as [k, v] (k)}<kbd>{k}</kbd><span>{v}</span>{/each}
      </div>
      <p>左右翻页时：点击屏幕左右两侧翻页、中间显示 / 隐藏工具栏；滚轮和触摸板滑动也能翻页。</p>
    </div>
  {/if}

  {#if selPop}
    <div class="mp-selpop open" style="left:{Math.max(8, selPop.x - 80)}px;top:{Math.max(8, selPop.y - 46)}px">
      <button type="button" onclick={() => markSelection(false)}>标记为广告</button>
      <button type="button" onclick={() => markSelection(true)}>标记，并找相似的清理全书</button>
    </div>
  {/if}
</div>

<style>
  .mp-rhead.nobar,
  .mp-rprog.nobar {
    display: none;
  }
  .nm-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .nm-btn svg {
    width: 15px;
    height: 15px;
  }
  .nm-btn.on {
    opacity: 1;
    background: rgba(128, 128, 128, 0.18);
  }
  .mp-rbody.pmode {
    overflow: hidden;
    padding-top: 18px;
    padding-bottom: 22px;
    display: flex;
    flex-direction: column;
  }
  .pview {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    cursor: default;
  }
  .pcols {
    position: relative;
    column-fill: auto;
    will-change: transform;
  }
  .pcols.slide {
    transition: transform 0.35s cubic-bezier(0.22, 0.7, 0.3, 1);
  }
  .pcols :global(h2) {
    margin-top: 0;
    break-after: avoid;
  }
  .pcols :global(.chapnav) {
    break-inside: avoid;
    margin: 30px 0 10px;
  }
  .autobar {
    position: absolute;
    left: 50%;
    bottom: 18px;
    transform: translateX(-50%);
    display: flex;
    gap: 8px;
    align-items: center;
    background: rgba(30, 30, 30, 0.82);
    color: #eee;
    border-radius: 12px;
    padding: 6px 10px;
    font-size: 12.5px;
    font-family: var(--mp-font);
    box-shadow: 0 8px 30px #0005;
    z-index: 5;
  }
  .autobar button {
    background: #ffffff18;
    border: 1px solid #ffffff2a;
    color: inherit;
    border-radius: 8px;
    padding: 3px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .autobar span {
    min-width: 5em;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .pinfo {
    position: absolute;
    right: 16px;
    bottom: 10px;
    font-size: 12px;
    opacity: 0.6;
    font-variant-numeric: tabular-nums;
    pointer-events: none;
  }
  .nhelp {
    position: absolute;
    top: 60px;
    right: 20px;
    width: min(440px, calc(100% - 40px));
    max-height: calc(100% - 80px);
    overflow: auto;
    background: #1b1b22f2;
    color: #eee;
    border: 1px solid #ffffff22;
    border-radius: 12px;
    padding: 14px 16px;
    box-shadow: 0 14px 40px #000a;
    z-index: 25;
    font-size: 12.5px;
    font-family: var(--mp-font);
  }
  .nhelp-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 10px;
  }
  .nhelp-head button {
    background: #ffffff14;
    border: 1px solid #ffffff26;
    color: inherit;
    border-radius: 8px;
    padding: 3px 10px;
    cursor: pointer;
  }
  .nhelp p {
    margin: 12px 0 0;
    opacity: 0.75;
    line-height: 1.6;
  }
  .nkv {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 12px;
    align-items: center;
  }
  .nkv kbd {
    font: inherit;
    font-size: 11.5px;
    background: #ffffff14;
    border: 1px solid #ffffff26;
    border-radius: 6px;
    padding: 1px 7px;
    white-space: nowrap;
    justify-self: start;
  }
  .toc {
    width: 0;
    flex: none;
    overflow: hidden;
    transition: width 0.3s;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .toc.open {
    width: 280px;
    border-right: 1px solid rgba(128, 128, 128, 0.3);
  }
  .hint {
    opacity: 0.6;
    padding: 20px;
  }
  .chapnav {
    display: flex;
    gap: 14px;
    align-items: center;
    justify-content: center;
    margin: 36px 0 70px;
    font-size: 13px;
    opacity: 0.85;
  }
  .chapnav button {
    border: 1px solid currentColor;
    background: none;
    color: inherit;
    border-radius: 8px;
    padding: 5px 12px;
    font-size: 12.5px;
    cursor: pointer;
  }
  .chapnav button:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .flash {
    animation: flash-para 1.4s ease-out;
  }
  @keyframes flash-para {
    from {
      background: rgba(255, 90, 60, 0.35);
    }
    to {
      background: transparent;
    }
  }
</style>
