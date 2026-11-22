<script lang="ts">
  // 滚动列表类歌词：逐行放大滚动 / 逐字高亮 / 弹性节拍 / 竖排 / 无动画逐行高亮（plain）。
  // 整个列表用 translate 移动，让当前行停在 pos 处；只在切行、尺寸变化时量一次位置，不逐帧触发布局。
  import type { LyricLine } from '../../lib/api';
  import { lyricsfx, lineUnits, unitProgress, lineProgress } from '../../lib/lyricsfx.svelte';

  let {
    lines,
    k,
    now,
    dur,
    mode,
    timed,
    pos = 0.42,
    beat = 0,
    onseek,
  }: {
    lines: LyricLine[];
    k: number;
    now: number;
    dur: number;
    mode: 'scroll' | 'karaoke' | 'bounce' | 'vertical' | 'plain';
    timed: boolean;
    pos?: number;
    beat?: number;
    onseek?: (t: number) => void;
  } = $props();

  let box = $state<HTMLDivElement>();
  let inner = $state<HTMLDivElement>();
  let shift = $state(0);
  let w = $state(0);
  let h = $state(0);
  const x = $derived(mode === 'vertical');
  const units = $derived(mode === 'karaoke' || mode === 'bounce' || mode === 'vertical' ? lineUnits(lines, k, dur) : []);
  const lp = $derived(mode === 'scroll' && k >= 0 ? lineProgress(lineUnits(lines, k, dur), now) : 0);

  // 没有时间轴的歌词：不移动，交给滚动条
  $effect(() => {
    void lines;
    void k;
    void w;
    void h;
    void x;
    if (!inner || !box || !timed) {
      shift = 0;
      return;
    }
    const el = inner.children[Math.max(k, 0)] as HTMLElement | undefined;
    if (!el) {
      shift = 0;
      return;
    }
    shift = x
      ? box.clientWidth * (1 - pos) - (inner.offsetLeft + el.offsetLeft + el.offsetWidth / 2)
      : box.clientHeight * pos - (inner.offsetTop + el.offsetTop + el.offsetHeight / 2);
  });

  // 字体、字号设置变化后行高会变，用 ResizeObserver 重新定位
  $effect(() => {
    if (!inner) return;
    const ro = new ResizeObserver(() => {
      w = inner?.offsetWidth ?? 0;
      h = inner?.offsetHeight ?? 0;
    });
    ro.observe(inner);
    if (box) ro.observe(box);
    return () => ro.disconnect();
  });
</script>

<div class="ly-box m-{mode}" class:free={!timed} data-mp="lyrics" data-axis={x ? 'x' : undefined} bind:this={box}>
  <div class="mp-ly-inner" bind:this={inner} style:transform={timed ? (x ? `translateX(${shift}px)` : `translateY(${shift}px)`) : undefined}>
    {#each lines as l, i}
      <div
        class="mp-ly-line"
        class:on={i === k}
        class:past={i < k}
        class:plain={!timed}
        style:--d={timed ? Math.min(Math.abs(i - k), 6) : 0}
        style:--lp={i === k ? lp : 0}
        style:--beat={i === k && mode === 'bounce' ? beat.toFixed(3) : undefined}
        role="button"
        tabindex="-1"
        onclick={() => timed && onseek?.(l.t)}
        onkeydown={() => {}}
      >
        {#if i === k && units.length}
          {#key k}
            <span class="ly-units">{#each units as u}{@const p = unitProgress(u, now)}<span class="u" class:hit={p > 0} class:done={p >= 1} style:--p={p > 0 && p < 1 ? p.toFixed(3) : p >= 1 ? 1 : 0}>{u.text}</span>{/each}</span>
          {/key}
        {:else}
          <span>{l.text}</span>
        {/if}
        {#if !x && lyricsfx.cfg.roma && l.roma}<small class="ly-sub roma">{l.roma}</small>{/if}
        {#if !x && lyricsfx.cfg.tr && l.tr}<small class="ly-sub">{l.tr}</small>{/if}
      </div>
    {/each}
  </div>
</div>
