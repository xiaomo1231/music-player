<script lang="ts">
  // 漫画 AI 翻译侧栏（由 mangatr 模块实现）
  let { comicId, page, pages, mode, onmode }: { comicId: number; page: number; pages: number; mode: 'orig' | 'tr' | 'both'; onmode: (m: 'orig' | 'tr' | 'both') => void } = $props();
</script>
<div class="rside"><h5>AI 翻译</h5><p style="color:#999;font-size:12px">建设中（{comicId} · {page + 1}/{pages} · {mode}）</p><button class="rbtn" onclick={() => onmode('orig')}>原文</button></div>
