<script lang="ts">
  import { coverUrl } from '../lib/api';
  import { coverCSS, hueOf } from '../lib/util';
  let { cover, name = '', cls = 'mp-cover', hue, alt = false, tag = 'span', children }: { cover?: string | null; name?: string; cls?: string; hue?: number | null; alt?: boolean; tag?: string; children?: import('svelte').Snippet } = $props();
  const bg = $derived(cover ? `url("${coverUrl(cover)}")` : coverCSS(hue ?? hueOf(name), alt));
</script>
<svelte:element this={tag} class={cls} style:background-image={bg}>{@render children?.()}</svelte:element>
