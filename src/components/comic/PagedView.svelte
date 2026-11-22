<script lang="ts">
  // 翻页模式（左右翻页 / 上下翻页）：同时排好上一组、当前组、下一组，翻页时滑动或仿真翻书。
  // 当前组超出窗口（放大、适应宽度）时可以在组内拖动 / 滚动查看；到边上再翻页。
  import { tick } from 'svelte';
  import { clamp } from '../../lib/util';
  import { spreadOf, spreadHeight, type ComicFit, type PageAnim } from '../../lib/readerprefs';
  import ComicPage from './ComicPage.svelte';

  let {
    id,
    spreads,
    page,
    axis,
    rtl,
    anim,
    fit,
    zoom,
    dimsVer,
    ratioOf,
    natOf,
    tr,
    ondims,
    onpage,
    onzoom,
    onmiddle,
  }: {
    id: number;
    spreads: number[][];
    page: number;
    axis: 'x' | 'y';
    rtl: boolean;
    anim: PageAnim;
    fit: ComicFit;
    zoom: number;
    dimsVer: number;
    ratioOf: (i: number) => number;
    natOf: (i: number) => number;
    tr: 'orig' | 'tr' | 'both';
    ondims: (i: number, w: number, h: number) => void;
    onpage: (i: number) => void;
    onzoom: (z: number) => void;
    onmiddle: () => void;
  } = $props();

  type Slot = { k: number; off: number; pages: { i: number; w: number; h: number }[] };

  let W = $state(0);
  let H = $state(0);
  let stageEl: HTMLDivElement | undefined = $state();
  /** 正在翻向哪边：+1 下一组 / -1 上一组 / 0 静止 */
  let delta = $state(0);
  /** 翻页进度 0-1 */
  let p = $state(0);
  /** 开着过渡动画 */
  let moving = $state(false);

  /** 动画结束时是否真的翻过去（拖到一半松手回弹时为 false） */
  let target = 0;
  let doneTimer: ReturnType<typeof setTimeout> | undefined;
  let tapTimer: ReturnType<typeof setTimeout> | undefined;
  let lastTap = { t: 0, x: 0, y: 0 };
  let ptr: { id: number; x: number; y: number; lx: number; ly: number; lt: number; v: number; d: number; mode: '' | 'swipe' | 'pan' } | null = null;
  // 滚轮
  let acc = 0;
  let lastEvt = 0;
  let lastNative = 0;
  let lastTurn = 0;
  let wheelLock = false;

  const index = $derived(spreadOf(spreads, page));
  /** 横向时“下一组”在哪边：从左到右在右边（+1），日漫在左边（-1） */
  const sign = $derived(axis === 'x' && rtl ? -1 : 1);
  const size = $derived(axis === 'x' ? W : H);
  const flip = $derived(anim === 'flip');
  const dur = $derived(flip ? 520 : 280);

  const slots = $derived.by<Slot[]>(() => {
    void dimsVer;
    const out: Slot[] = [];
    if (!W || !H) return out;
    for (const off of [-1, 0, 1]) {
      const k = index + off;
      if (k < 0 || k >= spreads.length) continue;
      const s = spreads[k];
      const rs = s.map(i => ratioOf(i));
      const nat = Math.max(...s.map(i => natOf(i)));
      const h = Math.floor(spreadHeight(fit, W, H, rs, nat) * zoom);
      out.push({ k, off, pages: s.map((i, j) => ({ i, w: Math.floor(h / rs[j]), h })) });
    }
    return out;
  });

  function slotStyle(s: Slot): string {
    const off = s.off;
    const tn = moving ? `transition:transform ${dur}ms cubic-bezier(.22,.7,.3,1),filter ${dur}ms;` : '';
    if (!flip) {
      const v = sign * (off - delta * p) * size;
      return `${tn}transform:${axis === 'x' ? `translate3d(${v}px,0,0)` : `translate3d(0,${v}px,0)`};`;
    }
    // 仿真翻书：翻走的那页绕书脊转过去，下面露出新的一页
    let angle = 0;
    let z = 1;
    let hidden = false;
    if (off === 0) {
      if (delta > 0) {
        angle = p * 180;
        z = 2;
      } else z = delta < 0 ? 1 : 2;
    } else if (off === 1) hidden = delta <= 0;
    else {
      hidden = delta >= 0;
      if (!hidden) {
        angle = (1 - p) * 180;
        z = 3;
      }
    }
    if (hidden) return 'visibility:hidden;z-index:0;';
    // 书脊在这一组页面的边上（页面比窗口小时居中，书脊不在窗口边上）
    const sw = s.pages.reduce((a, pg) => a + pg.w, 0);
    const sh = s.pages[0]?.h ?? H;
    const ex = Math.max(0, (W - sw) / 2);
    const ey = Math.max(0, (H - sh) / 2);
    const origin = axis === 'x' ? `${sign > 0 ? ex : W - ex}px 50%` : `50% ${ey}px`;
    const rot = axis === 'x' ? `rotateY(${-sign * angle}deg)` : `rotateX(${angle}deg)`;
    const shade = 1 - 0.45 * Math.sin((Math.min(angle, 180) * Math.PI) / 180);
    return `${tn}z-index:${z};transform-origin:${origin};transform:${rot};filter:brightness(${shade.toFixed(3)});`;
  }

  function curSlot(): HTMLElement | null {
    return stageEl?.querySelector<HTMLElement>('.slot.cur') ?? null;
  }

  /** 当前组在 ax 方向上还能不能往 d 的方向滚 */
  function canScroll(el: HTMLElement, ax: 'x' | 'y', d: number): boolean {
    if (ax === 'y') return d > 0 ? el.scrollTop + el.clientHeight < el.scrollHeight - 1 : d < 0 ? el.scrollTop > 0 : el.scrollHeight > el.clientHeight + 1;
    return d > 0 ? el.scrollLeft + el.clientWidth < el.scrollWidth - 1 : d < 0 ? el.scrollLeft > 0 : el.scrollWidth > el.clientWidth + 1;
  }

  const has = (dl: number) => index + dl >= 0 && index + dl < spreads.length;

  /** 翻完以后把新的一页滚到开头（往回翻时到结尾） */
  function afterTurn(dl: number) {
    void tick().then(() => {
      const el = curSlot();
      if (!el) return;
      el.scrollTop = dl < 0 && axis === 'y' ? el.scrollHeight : 0;
      el.scrollLeft = rtl ? el.scrollWidth : 0;
    });
  }

  function finish() {
    clearTimeout(doneTimer);
    const dl = delta;
    const go = target === 1 && dl !== 0 && has(dl);
    const k = index + dl;
    moving = false;
    delta = 0;
    p = 0;
    target = 0;
    if (go) {
      onpage(spreads[k][0]);
      afterTurn(dl);
    }
  }

  function animateTo(t: 0 | 1) {
    target = t;
    moving = true;
    requestAnimationFrame(() => {
      p = t;
      clearTimeout(doneTimer);
      doneTimer = setTimeout(finish, dur + 40);
    });
  }

  /** 翻页：dl 是阅读顺序上的方向，+1 为下一组。返回是否翻了 */
  export function turn(dl: number): boolean {
    if (delta !== 0 || moving) finish();
    if (!has(dl)) return false;
    if (anim === 'none' || !size) {
      onpage(spreads[index + dl][0]);
      afterTurn(dl);
      return true;
    }
    delta = dl;
    p = 0;
    void tick().then(() => animateTo(1));
    target = 1;
    return true;
  }

  /** 当前页超出窗口时先在页内滚动（方向键用），返回是否滚了 */
  export function scrollWithin(d: number): boolean {
    const el = curSlot();
    if (!el || !canScroll(el, 'y', d)) return false;
    el.scrollBy({ top: d * el.clientHeight * 0.6, behavior: 'smooth' });
    return true;
  }

  function zoomAt(z: number, cx: number, cy: number) {
    z = clamp(Math.round(z * 100) / 100, 0.5, 4);
    const el = curSlot();
    if (!el) return onzoom(z);
    const r = el.getBoundingClientRect();
    const px = cx - r.left;
    const py = cy - r.top;
    const fx = (el.scrollLeft + px) / el.scrollWidth;
    const fy = (el.scrollTop + py) / el.scrollHeight;
    onzoom(z);
    void tick().then(() => {
      const e2 = curSlot();
      if (!e2) return;
      e2.scrollLeft = fx * e2.scrollWidth - px;
      e2.scrollTop = fy * e2.scrollHeight - py;
    });
  }

  // ---------- 鼠标 / 触摸 ----------

  function onDown(e: PointerEvent) {
    if (e.button !== 0 || !stageEl) return;
    if (moving || delta !== 0) finish();
    ptr = { id: e.pointerId, x: e.clientX, y: e.clientY, lx: e.clientX, ly: e.clientY, lt: e.timeStamp, v: 0, d: 0, mode: '' };
    stageEl.setPointerCapture(e.pointerId);
  }

  function onMove(e: PointerEvent) {
    if (!ptr || e.pointerId !== ptr.id) return;
    const dx = e.clientX - ptr.x;
    const dy = e.clientY - ptr.y;
    const el = curSlot();
    if (!ptr.mode) {
      if (Math.hypot(dx, dy) < 6) return;
      const along = axis === 'x' ? Math.abs(dx) >= Math.abs(dy) : Math.abs(dy) >= Math.abs(dx);
      const cross = axis === 'x' ? 'y' : 'x';
      if (zoom > 1.001 || (el && along && canScroll(el, axis, 0)) || (el && !along && canScroll(el, cross, 0))) ptr.mode = 'pan';
      else ptr.mode = along ? 'swipe' : 'pan';
    }
    if (ptr.mode === 'pan') {
      if (el) {
        el.scrollLeft -= e.clientX - ptr.lx;
        el.scrollTop -= e.clientY - ptr.ly;
      }
    } else {
      const d = axis === 'x' ? dx : dy;
      const dt = Math.max(1, e.timeStamp - ptr.lt);
      ptr.v = (d - ptr.d) / dt;
      ptr.d = d;
      // 往负方向拖，露出正方向那一组
      const dl = d < 0 ? sign : -sign;
      let pp = size ? Math.min(1, Math.abs(d) / size) : 0;
      if (!has(dl)) pp = flip ? 0 : Math.min(0.08, pp * 0.25);
      delta = dl;
      p = pp;
    }
    ptr.lx = e.clientX;
    ptr.ly = e.clientY;
    ptr.lt = e.timeStamp;
  }

  function onUp(e: PointerEvent) {
    if (!ptr || e.pointerId !== ptr.id) return;
    const m = ptr.mode;
    const v = ptr.v;
    const d = ptr.d;
    ptr = null;
    if (!m) return onTap(e.clientX, e.clientY);
    if (m !== 'swipe') return;
    const fast = Math.abs(v) > 0.45 && Math.sign(v) === Math.sign(d);
    if (delta !== 0 && has(delta) && (p > 0.2 || fast)) animateTo(1);
    else animateTo(0);
  }

  function onTap(x: number, y: number) {
    const now = performance.now();
    if (now - lastTap.t < 300 && Math.hypot(x - lastTap.x, y - lastTap.y) < 30) {
      // 双击：放大 / 还原
      clearTimeout(tapTimer);
      lastTap.t = 0;
      zoomAt(zoom > 1.001 ? 1 : 2, x, y);
      return;
    }
    lastTap = { t: now, x, y };
    clearTimeout(tapTimer);
    tapTimer = setTimeout(() => tapZone(x, y), 260);
  }

  /** 点击区域：两边三分之一翻页，中间呼出 / 隐藏工具栏 */
  function tapZone(x: number, y: number) {
    if (!stageEl) return;
    const r = stageEl.getBoundingClientRect();
    const f = axis === 'x' ? (x - r.left) / r.width : (y - r.top) / r.height;
    if (f > 1 / 3 && f < 2 / 3) return onmiddle();
    if (zoom > 1.001) return;
    const side = f <= 1 / 3 ? -1 : 1;
    turn(axis === 'x' ? side * sign : side);
  }

  // ---------- 滚轮 ----------

  function onWheel(e: WheelEvent) {
    if (e.ctrlKey) {
      e.preventDefault();
      zoomAt(zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1), e.clientX, e.clientY);
      return;
    }
    const now = performance.now();
    const horiz = Math.abs(e.deltaX) > Math.abs(e.deltaY);
    const d = horiz ? e.deltaX : e.deltaY;
    const el = curSlot();
    // 页面超出窗口：先滚动页面本身
    if (el && canScroll(el, horiz ? 'x' : 'y', d)) {
      lastNative = now;
      return;
    }
    e.preventDefault();
    const gap = now - lastEvt;
    lastEvt = now;
    // 刚滚到边上：这一下滚动（含惯性）先不翻，停一下再滚才翻页
    if (now - lastNative < 350) {
      lastNative = now;
      return;
    }
    if (wheelLock) {
      // 翻过一页后，等滚轮 / 触摸板停一下再翻下一页
      if (gap > 180 || now - lastTurn > 700) wheelLock = false;
      else return;
    }
    if (gap > 250) acc = 0;
    acc += d;
    if (Math.abs(acc) < 30) return;
    const s = Math.sign(acc);
    acc = 0;
    lastTurn = now;
    wheelLock = true;
    turn(horiz && axis === 'x' ? s * sign : s);
  }

  // 舞台大小（自己用 ResizeObserver 量，切换翻页方式重新挂载时也可靠）
  $effect(() => {
    const el = stageEl;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      W = el.clientWidth;
      H = el.clientHeight;
    });
    ro.observe(el);
    W = el.clientWidth;
    H = el.clientHeight;
    return () => ro.disconnect();
  });

  $effect(() => {
    const el = stageEl;
    if (!el) return;
    const w = (e: WheelEvent) => onWheel(e);
    el.addEventListener('wheel', w, { passive: false });
    return () => el.removeEventListener('wheel', w);
  });

  $effect(() => () => {
    clearTimeout(doneTimer);
    clearTimeout(tapTimer);
  });
</script>

<div
  class="stage"
  class:zoomed={zoom > 1.001}
  bind:this={stageEl}
  onpointerdown={onDown}
  onpointermove={onMove}
  onpointerup={onUp}
  onpointercancel={onUp}
  role="presentation"
>
  {#each slots as s (s.k)}
    <div class="slot" class:cur={s.off === 0} style={slotStyle(s)}>
      <div class="inner" class:rev={rtl}>
        {#each s.pages as pg (pg.i)}
          <ComicPage {id} i={pg.i} w={pg.w} h={pg.h} {tr} {ondims} />
        {/each}
      </div>
    </div>
  {/each}
</div>

<style>
  .stage {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    perspective: 2400px;
    touch-action: none;
    user-select: none;
    background: #0d0d10;
  }
  .stage.zoomed {
    cursor: grab;
  }
  .slot {
    position: absolute;
    inset: 0;
    overflow: hidden;
    backface-visibility: hidden;
    will-change: transform;
    scrollbar-width: none;
  }
  .slot.cur {
    overflow: auto;
  }
  .inner {
    min-width: 100%;
    min-height: 100%;
    width: max-content;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .inner.rev {
    flex-direction: row-reverse;
  }
  .inner :global(.cpg) {
    box-shadow: 0 10px 40px #000a;
  }
</style>
