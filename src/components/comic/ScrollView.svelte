<script lang="ts">
  // 连续滚动模式（上下连续 = 条漫 / 长条；左右连续）：页面首尾相接，只渲染视口附近的页面。
  // “逻辑位置” L 从阅读起点量起：上下滚动就是 scrollTop；左右滚动从左到右是 scrollLeft，日漫从右端量起。
  import { untrack } from 'svelte';
  import { clamp } from '../../lib/util';
  import { offsets, pageAt, type ComicFit } from '../../lib/readerprefs';
  import ComicPage from './ComicPage.svelte';

  let {
    id,
    count,
    page,
    axis,
    rtl,
    fit,
    size,
    gap,
    zoom,
    dimsVer,
    ratioOf,
    natOf,
    natW,
    tr,
    ondims,
    onpage,
    onzoom,
    onmiddle,
  }: {
    id: number;
    count: number;
    page: number;
    axis: 'x' | 'y';
    rtl: boolean;
    fit: ComicFit;
    size: number;
    gap: number;
    zoom: number;
    dimsVer: number;
    ratioOf: (i: number) => number;
    natOf: (i: number) => number;
    natW: (i: number) => number;
    tr: 'orig' | 'tr' | 'both';
    ondims: (i: number, w: number, h: number) => void;
    onpage: (i: number) => void;
    onzoom: (z: number) => void;
    onmiddle: () => void;
  } = $props();

  const PAD = 16;

  let W = $state(0);
  let H = $state(0);
  let el: HTMLDivElement | undefined = $state();
  /** 逻辑滚动位置（渲染哪些页用，滚动时更新） */
  let L = $state(0);

  /** 锚点：第 i 页的 f 处对齐到视口里的 off 像素处；布局变化（图片加载、缩放、改宽度）后按它恢复位置 */
  let anc = { i: 0, f: 0, off: 0 };
  /** anc 是刚指定的（打开、换方向、缩放），下次布局变化按它定位；否则按变化前的布局从当前位置算锚点 */
  let pinned = false;
  let prevStart: number[] | null = null;
  let prevAlong: number[] = [];
  let started = false;
  let raf = 0;
  /** 平滑滚动的目标（逻辑位置），null 表示没有在平滑滚动 */
  let goal: number | null = null;
  let ptr: { id: number; x: number; y: number; lx: number; ly: number; lt: number; vx: number; vy: number; moved: boolean } | null = null;
  let tapTimer: ReturnType<typeof setTimeout> | undefined;
  let lastTap = { t: 0, x: 0, y: 0 };
  let inertia = 0;
  let lastRtl = untrack(() => rtl);

  const view = $derived(axis === 'y' ? H : W);

  /** 每页的显示尺寸 */
  const dims = $derived.by(() => {
    void dimsVer;
    const ws: number[] = [];
    const hs: number[] = [];
    if (!W || !H) return { ws, hs };
    for (let i = 0; i < count; i++) {
      const r = ratioOf(i);
      let w: number;
      if (axis === 'y') {
        if (fit === 'height') w = (H - PAD * 2) / r;
        else if (fit === 'original') w = natW(i) || (W * size) / 100;
        else w = ((W - PAD * 2) * size) / 100;
      } else {
        let h: number;
        if (fit === 'width') h = (((W - PAD * 2) * size) / 100) * r;
        else if (fit === 'original') h = natOf(i) || H - PAD * 2;
        else h = H - PAD * 2;
        w = h / r;
      }
      w = Math.max(20, Math.floor(w * zoom));
      ws.push(w);
      hs.push(Math.floor(w * r));
    }
    return { ws, hs };
  });

  const along = $derived(axis === 'y' ? dims.hs : dims.ws);
  const lay = $derived(offsets(along, gap));
  /** 内容区（含边距）的长、宽 */
  const mainLen = $derived(lay.total + PAD * 2);
  const crossLen = $derived.by(() => {
    const cs = axis === 'y' ? dims.ws : dims.hs;
    let m = 0;
    for (const c of cs) if (c > m) m = c;
    return Math.max(axis === 'y' ? W : H, m + PAD * 2);
  });

  /** 视口附近的页（前后各多留 1.5 屏） */
  const visible = $derived.by(() => {
    if (!count || !view) return [] as number[];
    const buf = view * 1.5;
    const a = pageAt(lay.start, L - PAD - buf);
    const b = pageAt(lay.start, L - PAD + view + buf);
    const out: number[] = [];
    for (let i = a; i <= b && i < count; i++) out.push(i);
    return out;
  });

  function pageStyle(i: number): string {
    const s = PAD + lay.start[i];
    const w = dims.ws[i];
    const h = dims.hs[i];
    if (axis === 'y') return `position:absolute;top:${s}px;left:${Math.floor((crossLen - w) / 2)}px;`;
    const x = rtl ? mainLen - s - w : s;
    return `position:absolute;left:${x}px;top:${Math.floor((crossLen - h) / 2)}px;`;
  }

  // ---------- 逻辑位置 <-> 滚动条 ----------

  function getL(): number {
    if (!el) return 0;
    if (axis === 'y') return el.scrollTop;
    return rtl ? el.scrollWidth - el.clientWidth - el.scrollLeft : el.scrollLeft;
  }

  function setL(v: number) {
    if (!el) return;
    v = clamp(v, 0, maxL());
    if (axis === 'y') el.scrollTop = v;
    else el.scrollLeft = rtl ? el.scrollWidth - el.clientWidth - v : v;
  }

  /** 实际的视口长度和最大滚动位置（直接读元素，滚动条出现 / 消失时也准确） */
  const viewNow = () => (el ? (axis === 'y' ? el.clientHeight : el.clientWidth) : view);
  const maxL = () => (el ? Math.max(0, axis === 'y' ? el.scrollHeight - el.clientHeight : el.scrollWidth - el.clientWidth) : 0);

  /** 按当前位置记下锚点、更新当前页 */
  function sync() {
    const l = getL();
    L = l;
    if (!count || !lay.start.length) return;
    const i = pageAt(lay.start, l - PAD);
    const len = along[i] || 1;
    anc = { i, f: (l - PAD - lay.start[i]) / len, off: 0 };
    // 当前页：视口上方四分之一处的那页；到底了算最后一页
    const cur = l >= maxL() - 2 && l > 0 ? count - 1 : pageAt(lay.start, l - PAD + viewNow() * 0.25);
    if (cur !== page) onpage(cur);
  }

  function onScroll() {
    if (raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      sync();
    });
  }

  // 布局变化后按锚点恢复位置（也负责第一次打开时跳到当前页）
  $effect(() => {
    void lay;
    void crossLen;
    void view;
    if (!el || !view || !lay.start.length) return;
    // 换阅读方向：滚动条的方向变了，正在进行的平滑滚动作废
    if (rtl !== lastRtl) {
      lastRtl = rtl;
      goal = null;
      pinned = true;
    }
    if (!started) {
      started = true;
      pinned = true;
      const k = clamp(untrack(() => page), 0, count - 1);
      anc = { i: k, f: 0, off: k === 0 ? PAD : 0 };
      if (axis === 'x') el.scrollTop = Math.max(0, (crossLen - H) / 2);
    }
    if (!pinned && prevStart && prevStart !== lay.start && prevStart.length) {
      const l = getL();
      const i = pageAt(prevStart, l - PAD);
      anc = { i, f: (l - PAD - prevStart[i]) / (prevAlong[i] || 1), off: 0 };
    }
    pinned = false;
    prevStart = lay.start;
    prevAlong = along;
    const a = anc;
    const want = PAD + (lay.start[a.i] ?? 0) + a.f * (along[a.i] ?? 0) - a.off;
    const now = getL();
    if (Math.abs(want - now) > 1) {
      if (goal != null) goal += want - now;
      setL(want);
    }
    L = getL();
  });

  // ---------- 平滑滚动 ----------

  function animate() {
    if (goal == null) return;
    const cur = getL();
    const g = clamp(goal, 0, maxL());
    const d = g - cur;
    if (Math.abs(d) < 0.8) {
      setL(g);
      goal = null;
      return;
    }
    setL(cur + d * 0.22);
    requestAnimationFrame(animate);
  }

  function smoothTo(v: number) {
    const run = goal == null;
    goal = clamp(v, 0, maxL());
    if (run) requestAnimationFrame(animate);
  }

  /** 按屏滚动：frac 为视口长度的倍数，正数往后 */
  export function scrollView(frac: number) {
    smoothTo((goal ?? getL()) + frac * viewNow());
  }

  /** 跳到第 k 页开头 */
  export function goto(k: number, smooth = false) {
    k = clamp(k, 0, Math.max(0, count - 1));
    const v = k === 0 ? 0 : PAD + lay.start[k];
    if (smooth) smoothTo(v);
    else {
      goal = null;
      anc = { i: k, f: 0, off: k === 0 ? PAD : 0 };
      setL(v);
      sync();
    }
  }

  /** 上一页 / 下一页（对齐到页首） */
  export function stepPage(d: number) {
    const l = goal ?? getL();
    let i = pageAt(lay.start, l - PAD + 2);
    if (d > 0) i = Math.min(count - 1, i + 1);
    else if (l - PAD - lay.start[i] < 4) i = Math.max(0, i - 1);
    goto(i, true);
  }

  function zoomAt(z: number, cx: number, cy: number) {
    z = clamp(Math.round(z * 100) / 100, 0.3, 4);
    if (!el) return onzoom(z);
    const r = el.getBoundingClientRect();
    const off = axis === 'y' ? cy - r.top : rtl ? r.right - cx : cx - r.left;
    const at = getL() + off - PAD;
    const i = pageAt(lay.start, at);
    anc = { i, f: (at - lay.start[i]) / (along[i] || 1), off: off - PAD };
    pinned = true;
    goal = null;
    onzoom(z);
  }

  // ---------- 鼠标 / 触摸 ----------

  function onDown(e: PointerEvent) {
    if (e.button !== 0 || !el) return;
    goal = null;
    cancelAnimationFrame(inertia);
    ptr = { id: e.pointerId, x: e.clientX, y: e.clientY, lx: e.clientX, ly: e.clientY, lt: e.timeStamp, vx: 0, vy: 0, moved: false };
  }

  function onMove(e: PointerEvent) {
    if (!ptr || e.pointerId !== ptr.id || !el) return;
    if (!ptr.moved) {
      if (Math.hypot(e.clientX - ptr.x, e.clientY - ptr.y) < 6) return;
      ptr.moved = true;
      el.setPointerCapture(e.pointerId);
    }
    const dx = e.clientX - ptr.lx;
    const dy = e.clientY - ptr.ly;
    const dt = Math.max(1, e.timeStamp - ptr.lt);
    ptr.vx = ptr.vx * 0.5 + (dx / dt) * 0.5;
    ptr.vy = ptr.vy * 0.5 + (dy / dt) * 0.5;
    el.scrollLeft -= dx;
    el.scrollTop -= dy;
    ptr.lx = e.clientX;
    ptr.ly = e.clientY;
    ptr.lt = e.timeStamp;
  }

  function onUp(e: PointerEvent) {
    if (!ptr || e.pointerId !== ptr.id) return;
    const p = ptr;
    ptr = null;
    if (!p.moved) return onTap(e.clientX, e.clientY);
    // 松手后的惯性
    let vx = p.vx * 16;
    let vy = p.vy * 16;
    if (e.timeStamp - p.lt > 80) return;
    const step = () => {
      if (!el || (Math.abs(vx) < 0.5 && Math.abs(vy) < 0.5)) return;
      el.scrollLeft -= vx;
      el.scrollTop -= vy;
      vx *= 0.93;
      vy *= 0.93;
      inertia = requestAnimationFrame(step);
    };
    inertia = requestAnimationFrame(step);
  }

  function onTap(x: number, y: number) {
    const now = performance.now();
    if (now - lastTap.t < 300 && Math.hypot(x - lastTap.x, y - lastTap.y) < 30) {
      clearTimeout(tapTimer);
      lastTap.t = 0;
      zoomAt(zoom > 1.001 ? 1 : 1.6, x, y);
      return;
    }
    lastTap = { t: now, x, y };
    clearTimeout(tapTimer);
    tapTimer = setTimeout(() => tapZone(x, y), 260);
  }

  /** 点击区域：上下（左右）三分之一滚动一屏，中间呼出 / 隐藏工具栏 */
  function tapZone(x: number, y: number) {
    if (!el) return;
    const r = el.getBoundingClientRect();
    const f = axis === 'y' ? (y - r.top) / r.height : (x - r.left) / r.width;
    if (f > 1 / 3 && f < 2 / 3) return onmiddle();
    let d = f <= 1 / 3 ? -1 : 1;
    if (axis === 'x' && rtl) d = -d;
    scrollView(d * 0.85);
  }

  function onWheel(e: WheelEvent) {
    if (e.ctrlKey) {
      e.preventDefault();
      zoomAt(zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1), e.clientX, e.clientY);
      return;
    }
    // 左右连续滚动：竖向滚轮换成横向
    if (axis === 'x' && Math.abs(e.deltaY) > Math.abs(e.deltaX) && !e.shiftKey) {
      e.preventDefault();
      const px = e.deltaMode === 1 ? e.deltaY * 40 : e.deltaY;
      smoothTo((goal ?? getL()) + px * 1.2);
      return;
    }
    goal = null;
  }

  // 视口大小（自己用 ResizeObserver 量，切换翻页方式重新挂载时也可靠）
  $effect(() => {
    const e = el;
    if (!e) return;
    const ro = new ResizeObserver(() => {
      W = e.clientWidth;
      H = e.clientHeight;
    });
    ro.observe(e);
    W = e.clientWidth;
    H = e.clientHeight;
    return () => ro.disconnect();
  });

  $effect(() => {
    const e = el;
    if (!e) return;
    const w = (ev: WheelEvent) => onWheel(ev);
    e.addEventListener('wheel', w, { passive: false });
    return () => e.removeEventListener('wheel', w);
  });

  $effect(() => () => {
    cancelAnimationFrame(raf);
    cancelAnimationFrame(inertia);
    clearTimeout(tapTimer);
    goal = null;
  });

</script>

<div
  class="sv"
  class:x={axis === 'x'}
  bind:this={el}
  onscroll={onScroll}
  onpointerdown={onDown}
  onpointermove={onMove}
  onpointerup={onUp}
  onpointercancel={onUp}
  role="presentation"
>
  <div class="track" style={axis === 'y' ? `height:${mainLen}px;width:${crossLen}px` : `width:${mainLen}px;height:${crossLen}px`}>
    {#each visible as i (i)}
      <ComicPage {id} {i} w={dims.ws[i]} h={dims.hs[i]} {tr} {ondims} style={pageStyle(i)} />
    {/each}
  </div>
</div>

<style>
  .sv {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: auto;
    user-select: none;
    touch-action: none;
    background: #0d0d10;
    scrollbar-width: thin;
    scrollbar-color: #ffffff40 transparent;
  }
  .track {
    position: relative;
  }
</style>
