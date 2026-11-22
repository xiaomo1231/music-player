<script lang="ts">
  // 漫画的一页：固定尺寸的盒子里放图片，译文层叠在上面（PageOverlay 需要父元素 position:relative）
  import { comicPage } from '../../lib/api';
  import PageOverlay from '../mangatr/PageOverlay.svelte';

  let {
    id,
    i,
    w,
    h,
    tr,
    ondims,
    style = '',
  }: {
    id: number;
    i: number;
    w: number;
    h: number;
    tr: 'orig' | 'tr' | 'both';
    ondims: (i: number, w: number, h: number) => void;
    style?: string;
  } = $props();

  let loaded = $state(false);

  function onload(e: Event) {
    const im = e.currentTarget as HTMLImageElement;
    loaded = true;
    if (im.naturalWidth && im.naturalHeight) ondims(i, im.naturalWidth, im.naturalHeight);
  }
</script>

<div class="cpg" class:loaded style="width:{w}px;height:{h}px;{style}" data-i={i}>
  <img src={comicPage(id, i)} alt="第 {i + 1} 页" draggable="false" decoding="async" {onload} />
  {#if tr !== 'orig'}
    <PageOverlay comicId={id} page={i} mode={tr} />
  {/if}
</div>

<style>
  .cpg {
    position: relative;
    flex: none;
    background: #18181d;
  }
  .cpg.loaded {
    background: transparent;
  }
  .cpg img {
    display: block;
    width: 100%;
    height: 100%;
    user-select: none;
    -webkit-user-drag: none;
    pointer-events: none;
  }
</style>
