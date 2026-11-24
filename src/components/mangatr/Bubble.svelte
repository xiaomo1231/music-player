<script lang="ts">
  // 一个气泡：白底（取样底色）盖住原文，再把译文按气泡大小排进去；mode='both' 时在下方显示原文
  import { fitText, textColor } from './layout';
  import type { Block } from '../../lib/mangatr.svelte';

  let { block, mode, selected = false }: { block: Block; mode: 'tr' | 'both'; selected?: boolean } = $props();

  let box = $state<HTMLDivElement>();
  let W = $state(0);
  let H = $state(0);

  $effect(() => {
    const el = box;
    if (!el) return;
    const measure = () => {
      W = el.clientWidth;
      H = el.clientHeight;
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  });

  const fit = $derived(block.tr.trim() && W > 0 && H > 0 ? fitText(block.tr, W, H, block.vertical) : null);
  const bg = $derived(block.bg || '#ffffff');
  const color = $derived(textColor(bg));
</script>

<div
  class="mtr-b"
  class:mtr-sel={selected}
  style="left:{block.bbox[0] * 100}%;top:{block.bbox[1] * 100}%;width:{block.bbox[2] * 100}%;height:{block.bbox[3] * 100}%"
>
  {#if block.tr.trim()}
    <div class="mtr-cover" style="background:{bg}"></div>
    <div class="mtr-t" bind:this={box} style="color:{color};font-size:{fit?.size ?? 12}px">
      {#if block.vertical}
        <div class="mtr-cols">
          {#each fit?.lines ?? [] as col}<span class="mtr-col">{col}</span>{/each}
        </div>
      {:else}
        {#each fit?.lines ?? [] as line}<div>{line}</div>{/each}
      {/if}
    </div>
  {/if}
  {#if mode === 'both'}
    <div class="mtr-src">{block.src}</div>
  {/if}
</div>

<style>
  .mtr-b {
    position: absolute;
    pointer-events: none;
  }
  .mtr-b.mtr-sel {
    outline: 2px dashed #ff3d6e;
    outline-offset: 2px;
  }
  .mtr-cover {
    position: absolute;
    inset: -2px;
    border-radius: 6px;
  }
  .mtr-t {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    justify-content: center;
    align-items: center;
    text-align: center;
    line-height: 1.25;
    font-family: "Microsoft YaHei UI", "Microsoft YaHei", "PingFang SC", "Noto Sans SC", sans-serif;
    overflow: hidden;
  }
  .mtr-cols {
    display: flex;
    flex-direction: row-reverse;
    justify-content: center;
    align-items: center;
    height: 100%;
    width: 100%;
  }
  .mtr-col {
    writing-mode: vertical-rl;
    line-height: 1.2;
    white-space: nowrap;
  }
  .mtr-src {
    position: absolute;
    top: calc(100% + 2px);
    left: 50%;
    translate: -50% 0;
    white-space: nowrap;
    font-size: 10px;
    background: #111;
    color: #fff;
    padding: 1px 6px;
    border-radius: 4px;
    z-index: 2;
  }
</style>
