<script module lang="ts">
  export interface DmItem {
    start: number;
    end: number;
    mode: 'scroll' | 'top' | 'bottom' | string;
    /** 纵向位置，占画面高度的比例 */
    y: number;
    color: string;
    scale: number;
    text: string;
  }
</script>

<script lang="ts">
  // 弹幕层：按视频当前时间绘制视频旁边 .danmaku.ass 里的弹幕（滚动 / 顶部 / 底部）。
  // 位置和轨道在下载时已经排好（见后端 danmaku.rs），这里只负责按时间移动。
  import { onDestroy } from 'svelte';

  let { video, items, on, opacity = 0.9 }: { video: HTMLVideoElement | undefined; items: DmItem[]; on: boolean; opacity?: number } = $props();

  let box = $state<HTMLDivElement>();
  let w = $state(0);
  let h = $state(0);
  let now = $state(0);
  let raf = 0;

  /** 同一时刻最多画这么多条，太密时保护性能 */
  const MAX = 120;
  /** 按出现时间排好序（二分查找要用） */
  const list = $derived([...items].sort((a, b) => a.start - b.start));
  /** 最长一条弹幕的持续时间，用来确定往前找的范围 */
  const maxDur = $derived(list.reduce((m, d) => Math.max(m, d.end - d.start), 0));

  // 第一条 start >= t 的位置
  function lowerBound(t: number): number {
    let lo = 0;
    let hi = list.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (list[mid].start < t) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  const active = $derived.by(() => {
    if (!on || !list.length) return [] as { d: DmItem; i: number; p: number }[];
    const t = now;
    const out: { d: DmItem; i: number; p: number }[] = [];
    const to = lowerBound(t + 1e-6);
    for (let i = lowerBound(t - maxDur); i < to && out.length < MAX; i++) {
      const d = list[i];
      if (d.end <= t) continue;
      out.push({ d, i, p: (t - d.start) / (d.end - d.start) });
    }
    return out;
  });

  function frame() {
    if (video) now = video.currentTime;
    raf = requestAnimationFrame(frame);
  }

  $effect(() => {
    if (on && items.length && video) {
      raf = requestAnimationFrame(frame);
      return () => cancelAnimationFrame(raf);
    }
  });

  $effect(() => {
    if (!box) return;
    const ro = new ResizeObserver(() => {
      w = box?.clientWidth ?? 0;
      h = box?.clientHeight ?? 0;
    });
    ro.observe(box);
    return () => ro.disconnect();
  });

  onDestroy(() => cancelAnimationFrame(raf));

  // 1080 高的画面上基准字号 48
  const font = $derived(Math.max(14, (h * 48) / 1080));
</script>

<div class="dm" bind:this={box} style:opacity aria-hidden="true">
  {#each active as a (a.i)}
    {#if a.d.mode === 'scroll'}
      <span
        class="c"
        style:top="{a.d.y * 100}%"
        style:color={a.d.color}
        style:font-size="{font * a.d.scale}px"
        style:transform="translateX(calc({w * (1 - a.p)}px - {a.p * 100}%))"
      >{a.d.text}</span>
    {:else}
      <span
        class="c fixed"
        style:top="{a.d.y * 100}%"
        style:color={a.d.color}
        style:font-size="{font * a.d.scale}px"
      >{a.d.text}</span>
    {/if}
  {/each}
</div>

<style>
  .dm {
    position: absolute;
    inset: 0;
    overflow: hidden;
    pointer-events: none;
    z-index: 2;
  }
  .c {
    position: absolute;
    left: 0;
    white-space: pre;
    font-weight: 700;
    line-height: 1.15;
    text-shadow:
      0 0 2px #000,
      1px 1px 2px #000;
    will-change: transform;
  }
  .fixed {
    left: 50%;
    transform: translateX(-50%);
  }
</style>
