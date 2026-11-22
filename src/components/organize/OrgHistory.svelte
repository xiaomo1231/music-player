<script lang="ts">
  import { MODE_LABEL, type OrgBatch } from '../../lib/organize';
  import { fmtDate } from '../../lib/util';

  let { batches, busy, onundo }: { batches: OrgBatch[]; busy: boolean; onundo: (id: number) => void } = $props();
</script>

<div class="og">历史批次 <span class="badge2">{batches.length}</span></div>
{#if batches.length}
  {#each batches as b (b.id)}
    <div class="orow" class:skip={b.undone}>
      <span class="from">#{b.id} · {fmtDate(b.created_at)} · {MODE_LABEL[b.mode] ?? b.mode}</span>
      <span class="badge2">{b.undone ? '已撤销' : `${b.count} 个文件`}</span>
      {#if !b.undone && b.mode !== 'library'}
        <button class="mp-btn ghost" disabled={busy} onclick={() => onundo(b.id)}>撤销这一批</button>
      {/if}
    </div>
  {/each}
{:else}
  <div class="note">还没有整理过文件</div>
{/if}
