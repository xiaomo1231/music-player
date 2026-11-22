<script lang="ts">
  import { P } from './icons';
  let { name, size }: { name: string; size?: string } = $props();
</script>
<svg class="mp-ico" style:font-size={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">{@html P[name] ?? ''}</svg>
