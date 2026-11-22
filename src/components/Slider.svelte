<script lang="ts">
  // 可拖动的进度 / 音量条（沿用原型的 .line 样式）
  import { clamp } from '../lib/util';
  let { value, onchange, cls = 'line', title = '', mp }: { value: number; onchange: (v: number) => void; cls?: string; title?: string; mp?: string } = $props();
  let el: HTMLDivElement;
  function at(e: PointerEvent) {
    const r = el.getBoundingClientRect();
    onchange(clamp((e.clientX - r.left) / r.width));
  }
</script>
<div bind:this={el} class={cls} data-mp={mp} {title} style:--p={value} role="slider" tabindex="-1" aria-valuenow={value}
  onpointerdown={e => { el.setPointerCapture(e.pointerId); at(e); }}
  onpointermove={e => { if (el.hasPointerCapture(e.pointerId)) at(e); }}></div>
