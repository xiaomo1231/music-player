<script lang="ts">
  // 卡片右下角的“⋯”按钮（和歌曲行最右边的样式一致）
  import Icon from '../lib/Icon.svelte';
  let { onopen, title = '更多', floating = true }: { onopen: (e: MouseEvent) => void; title?: string; floating?: boolean } = $props();
</script>

<button class="morebtn" class:floating {title} aria-label={title} onclick={e => { e.stopPropagation(); onopen(e); }}
  onpointerdown={e => e.stopPropagation()}><Icon name="more" /></button>

<style>
  .morebtn { background: none; border: 0; padding: 3px; font-size: 14px; border-radius: 6px; opacity: .75; color: inherit; line-height: 0; }
  .morebtn:hover { opacity: 1; background: rgba(255, 255, 255, .12); }
  .morebtn.floating { position: absolute; right: 6px; bottom: 6px; z-index: 3; background: rgba(0, 0, 0, .55); opacity: 0; transition: opacity .15s; }
  :global(.mp-card:hover) .morebtn.floating, :global(.ep:hover) .morebtn.floating, .morebtn.floating:focus-visible { opacity: 1; }
  .morebtn.floating:hover { background: rgba(0, 0, 0, .75); }
</style>
