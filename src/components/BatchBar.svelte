<script lang="ts">
  // 多选时固定在列表上方的批量操作栏：已选数量、全选、各项操作、完成
  import type { MenuEntry } from '../lib/items.svelte';
  import type { Selection } from '../lib/select.svelte';
  let {
    sel,
    unit,
    total,
    onall,
    entries,
    hint = '单击勾选，Shift 连选，Ctrl+A 全选',
  }: { sel: Selection; unit: string; total: number; onall: () => void; entries: MenuEntry[]; hint?: string } = $props();
</script>

<div class="batchbar" role="toolbar" aria-label="批量操作">
  <div class="acts">
    <b>已选 {sel.size} {unit}</b>
    {#if sel.size < total}
      <button class="mp-chip" onclick={onall}>全选（{total}）</button>
    {:else}
      <button class="mp-chip" onclick={() => (sel.set = new Set())}>全不选</button>
    {/if}
    <i class="sep"></i>
    {#if sel.size}
      {#each entries as en}
        {#if en !== 'sep'}<button class="mp-chip" class:danger={en.danger} onclick={() => en.run()}>{en.label}</button>{/if}
      {/each}
    {:else}
      <span class="hint">{hint}</span>
    {/if}
  </div>
  <button class="mp-chip done" title="退出多选（Esc）" onclick={() => sel.clear()}>完成</button>
</div>

<style>
  .batchbar { position: sticky; top: 0; z-index: 8; display: flex; align-items: flex-start; gap: 10px; margin: 12px 0 8px; padding: 8px 10px 8px 14px;
    border-radius: 14px; background: linear-gradient(var(--mp-panel), var(--mp-panel)), #121318; border: 1px solid var(--mp-accent); box-shadow: 0 8px 26px rgba(0, 0, 0, .35); backdrop-filter: blur(30px); }
  .batchbar .acts { flex: 1; min-width: 0; display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .batchbar b { font-size: 13.5px; margin-right: 4px; white-space: nowrap; color: var(--mp-accent); }
  .batchbar .mp-chip { padding: 4px 11px; white-space: nowrap; }
  .batchbar .mp-chip:hover { border-color: var(--mp-accent); }
  .batchbar .mp-chip.danger { color: #ff8a8a; }
  .batchbar .mp-chip.danger:hover { border-color: #ff8a8a; }
  .batchbar .sep { width: 1px; height: 18px; background: var(--edge); margin: 0 2px; }
  .batchbar .hint { font-size: 12px; color: var(--mp-sub); }
  .batchbar .done { flex: none; }
  .batchbar .hint { line-height: 26px; }
</style>
