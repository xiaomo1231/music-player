<script lang="ts">
  import { tick } from 'svelte';
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
  import AdPanel from '../components/reader/AdPanel.svelte';
  import TocPanel from '../components/reader/TocPanel.svelte';

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

  /** 当前加载的是哪本书（非响应式，给保存进度用） */
  let loadedId = 0;
  let saveTimer: ReturnType<typeof setTimeout> | undefined;

  const chapter = $derived(book && book.chapters[ci] ? book.chapters[ci] : null);
  const total = $derived(book ? book.chapters.length : 0);
  const pct = $derived(total ? ((ci + ratio) / total) * 100 : 0);
  const appliedCount = $derived(hits.filter(h => h.status === 'applied').length);
  const pStyle = $derived(`line-height:${prefs.lh};font-family:${FONTS[prefs.font]}`);

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
    el.addEventListener('mouseup', up);
    el.addEventListener('scroll', sc, { passive: true });
    return () => {
      el.removeEventListener('mouseup', up);
      el.removeEventListener('scroll', sc);
    };
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
      const [b, items, marks, hs] = await Promise.all([
        invoke<Book>('open_book', { id: bid }),
        invoke<BookItem[]>('list_books'),
        loadBookmarks(bid),
        invoke<AdHit[]>('adclean_list', { id: bid }),
      ]);
      if (loadedId !== bid) return;
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
    const el = bodyEl;
    if (!el) return ratio;
    const max = el.scrollHeight - el.clientHeight;
    return max > 0 ? Math.min(1, el.scrollTop / max) : 0;
  }

  function applyScroll() {
    const el = bodyEl;
    if (!el) return;
    el.scrollTop = ratio * Math.max(0, el.scrollHeight - el.clientHeight);
  }

  function saveNow() {
    if (!loadedId || !book || !inTauri) return;
    const r = bodyEl ? currentRatio() : ratio;
    invoke('save_book_progress', { id: loadedId, chapter: ci, offset: r, chapters: book.chapters.length }).catch(() => {});
  }

  function onScroll() {
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
    document.getElementById(`p-${c}-${p}`)?.scrollIntoView({ block: 'center' });
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
  function visibleLabel(): string {
    const el = bodyEl;
    if (el) {
      const top = el.getBoundingClientRect().top;
      for (const p of el.querySelectorAll<HTMLElement>('p[data-p]')) {
        if (p.getBoundingClientRect().bottom > top + 4) return cps((p.textContent ?? '').trim()).slice(0, 18).join('');
      }
    }
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
    const el = bodyEl;
    if (!el) return 0;
    const top = el.getBoundingClientRect().top;
    for (const p of el.querySelectorAll<HTMLElement>('p[data-p]')) {
      if (p.getBoundingClientRect().bottom > top + 4) return Number(p.dataset.p);
    }
    return 0;
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
    document.getElementById(`p-${ci}-${pi}`)?.scrollIntoView({ block: 'center', behavior: 'smooth' });
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
      if (selPop) selPop = null;
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
    switch (e.key) {
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
      case 'Home':
        e.preventDefault();
        bodyEl?.scrollTo({ top: 0 });
        break;
      case 'End':
        e.preventDefault();
        if (bodyEl) bodyEl.scrollTo({ top: bodyEl.scrollHeight });
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

<div class="mp-over mp-reader open" class:green={prefs.theme === 'green'} class:night={prefs.theme === 'night'} class:adopen={adOpen} class:showraw={raw}>
  <div class="mp-rhead">
    <button type="button" onclick={close}>← 书架</button>
    <b>{book?.title ?? '小说'}</b>
    <span>{chapter ? chapter.title : ''}</span>
    <span style="flex:1"></span>
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
  </div>
  <div class="mp-rprog"><i style="width:{pct}%"></i></div>

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

    <div class="mp-rbody" bind:this={bodyEl}>
      {#if loading}
        <p class="hint">正在打开…</p>
      {:else if err}
        <p class="hint">{err}</p>
      {:else if chapter}
        <h2>{chapter.title}</h2>
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
      {/if}
    </div>

    <div class="mp-rside">
      <AdPanel {hits} {candidates} {busy} {estimate} {raw} {scanned} act={onAct} />
    </div>
  </div>

  {#if selPop}
    <div class="mp-selpop open" style="left:{Math.max(8, selPop.x - 80)}px;top:{Math.max(8, selPop.y - 46)}px">
      <button type="button" onclick={() => markSelection(false)}>标记为广告</button>
      <button type="button" onclick={() => markSelection(true)}>标记，并找相似的清理全书</button>
    </div>
  {/if}
</div>

<style>
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
