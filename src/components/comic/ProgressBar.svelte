<script lang="ts">
  // 底部进度条：拖动或点击跳页，悬停 / 拖动时显示那一页的缩略图和页码。日漫横向阅读时方向反过来
  import { comicPage } from '../../lib/api';
  import { clamp } from '../../lib/util';

  let { id, count, page, reverse, onseek }: { id: number; count: number; page: number; reverse: boolean; onseek: (k: number) => void } = $props();

  let track: HTMLDivElement | undefined = $state();
  let tip = $state<{ x: number; k: number } | null>(null);
  let drag = $state(false);

  const frac = (k: number) => (count > 1 ? k / (count - 1) : 1);
  const shown = $derived(drag && tip ? tip.k : page);
  const pct = $derived(frac(shown) * 100);

  function at(e: PointerEvent): { x: number; k: number } | null {
    if (!track || !count) return null;
    const r = track.getBoundingClientRect();
    const x = clamp(e.clientX - r.left, 0, r.width);
    let f = r.width ? x / r.width : 0;
    if (reverse) f = 1 - f;
    return { x, k: clamp(Math.round(f * (count - 1)), 0, count - 1) };
  }

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    drag = true;
    tip = at(e);
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function move(e: PointerEvent) {
    tip = at(e);
  }

  function up(e: PointerEvent) {
    if (!drag) return;
    drag = false;
    const t = at(e);
    if (t) onseek(t.k);
  }
</script>

<div
  class="cpb"
  class:drag
  bind:this={track}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={() => (drag = false)}
  onpointerleave={() => !drag && (tip = null)}
  role="slider"
  tabindex="-1"
  aria-label="阅读进度"
  aria-valuemin={1}
  aria-valuemax={count}
  aria-valuenow={page + 1}
>
  <div class="rail">
    <i class="fill" style={reverse ? `right:0;width:${pct}%` : `left:0;width:${pct}%`}></i>
    <i class="knob" style={reverse ? `right:${pct}%` : `left:${pct}%`}></i>
  </div>
  {#if tip}
    <div class="tip" style="left:{tip.x}px">
      <img src={comicPage(id, tip.k)} alt="" draggable="false" />
      <span>{tip.k + 1} / {count}</span>
    </div>
  {/if}
</div>

<style>
  .cpb {
    position: relative;
    flex: 1;
    min-width: 0;
    height: 26px;
    display: flex;
    align-items: center;
    cursor: pointer;
    touch-action: none;
    outline: none;
  }
  .rail {
    position: relative;
    width: 100%;
    height: 4px;
    border-radius: 3px;
    background: #ffffff26;
  }
  .cpb:hover .rail,
  .cpb.drag .rail {
    height: 6px;
  }
  .fill {
    position: absolute;
    top: 0;
    bottom: 0;
    border-radius: 3px;
    background: linear-gradient(90deg, #ff3d6e, #ff8a3d);
  }
  .knob {
    position: absolute;
    top: 50%;
    width: 14px;
    height: 14px;
    margin: -7px -7px 0;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 6px #0008;
  }
  .tip {
    position: absolute;
    bottom: 30px;
    transform: translateX(-50%);
    background: #1b1b22;
    border: 1px solid #ffffff22;
    border-radius: 10px;
    padding: 6px;
    display: grid;
    gap: 4px;
    justify-items: center;
    pointer-events: none;
    box-shadow: 0 8px 30px #000c;
    z-index: 5;
  }
  .tip img {
    display: block;
    width: 96px;
    height: 134px;
    object-fit: contain;
    background: #0d0d10;
    border-radius: 4px;
  }
  .tip span {
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: #ddd;
  }
</style>
