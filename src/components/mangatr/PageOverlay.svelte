<script lang="ts">
  // 叠在漫画页面上的译文层（由 mangatr 模块实现）。父元素需要 position:relative，图片铺满父元素
  let { comicId, page, mode }: { comicId: number; page: number; mode: 'orig' | 'tr' | 'both' } = $props();
</script>
<div data-comic={comicId} data-page={page} data-mode={mode} style="position:absolute;inset:0;pointer-events:none"></div>
