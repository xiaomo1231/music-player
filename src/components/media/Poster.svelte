<script lang="ts">
  import type { Snippet } from 'svelte';
  import { posterBg } from '../../lib/media.svelte';

  let {
    poster = null,
    thumb = null,
    name = '',
    cls = 'mp-poster',
    alt = false,
    children,
  }: { poster?: string | null; thumb?: string | null; name?: string; cls?: string; alt?: boolean; children?: Snippet } = $props();

  const bg = $derived(posterBg(poster, thumb, name, alt));
</script>

<div class={cls} style:background-image={bg}>{@render children?.()}</div>
