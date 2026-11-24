<script lang="ts">
  import type { Snippet } from 'svelte';
  let { title, onclose, wide = false, top = false, children }: { title: string; onclose: () => void; wide?: boolean; top?: boolean; children: Snippet } = $props();
  let open = $state(false);
  $effect(() => {
    requestAnimationFrame(() => (open = true));
  });
  function key(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.stopPropagation();
      onclose();
    }
  }
</script>

<svelte:window onkeydown={key} />
<div class="mp-modal" class:open class:top role="presentation" onpointerdown={e => e.target === e.currentTarget && onclose()}>
  <div class="mp-sheet" class:wide>
    <h3>{title}<button class="mp-x" onclick={onclose}>✕</button></h3>
    {@render children()}
  </div>
</div>
