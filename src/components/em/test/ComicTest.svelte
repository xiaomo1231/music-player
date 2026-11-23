<script lang="ts">
  // 规则管理页“测试”面板（Comic）
  import type { EmRule } from '../../../lib/em.svelte';
  let { rule }: { rule: EmRule } = $props();
</script>

<p class="note">「{rule.name}」暂时没有测试面板。</p>
