<script lang="ts">
  // 叠在漫画页面上的译文层。父元素需要 position:relative，图片铺满父元素。
  // 只显示有译文、非拟声词 / 画面文字的气泡；译文按百分比坐标画在原气泡位置。
  import { untrack } from 'svelte';
  import { mangatr } from '../../lib/mangatr.svelte';
  import Bubble from './Bubble.svelte';

  let { comicId, page, mode }: { comicId: number; page: number; mode: 'orig' | 'tr' | 'both' } = $props();

  $effect(() => {
    const id = comicId;
    untrack(() => void mangatr.load(id));
  });

  const blocks = $derived(mangatr.comicId === comicId ? mangatr.blocksOf(page).filter((b) => !b.skipped && b.tr.trim() !== '') : []);
</script>

{#if mode !== 'orig'}
  <div class="mtr-layer" data-comic={comicId} data-page={page} data-mode={mode}>
    {#each blocks as b (b.id)}
      <Bubble block={b} mode={mode === 'both' ? 'both' : 'tr'} selected={mangatr.selected === b.id} />
    {/each}
  </div>
{/if}

<style>
  .mtr-layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
</style>
