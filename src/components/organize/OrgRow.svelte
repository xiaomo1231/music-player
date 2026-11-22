<script lang="ts">
  import { CONF, type OrgItem } from '../../lib/organize';

  let { item, done = false }: { item: OrgItem; done?: boolean } = $props();

  let editing = $state(false);
  let draft = $state('');

  function startEdit() {
    if (!item.dst || done) return;
    draft = item.dst;
    editing = true;
  }

  function commit() {
    if (!editing) return;
    editing = false;
    const v = draft.trim();
    if (!v || v === item.dst) return;
    item.dst = v;
  }

  function cancel() {
    editing = false;
  }

  const badge = $derived(!item.dst ? { cls: '', label: '未识别' } : (CONF[item.confidence] ?? { cls: '', label: '' }));
</script>

<div class="orow" class:done class:skip={!item.dst}>
  <input type="checkbox" bind:checked={item.checked} disabled={!item.dst || done} />
  <code class="from" title={item.src}>{item.src}</code>
  <span class="badge2 {badge.cls}">{badge.label}</span>
  {#if editing}
    <input
      class="sel"
      style="grid-column:2;min-width:0;width:100%"
      bind:value={draft}
      onblur={commit}
      onkeydown={e => {
        if (e.key === 'Enter') commit();
        if (e.key === 'Escape') cancel();
      }}
    />
  {:else}
    <button type="button" class="to-btn" title={item.dst ? '点一下修改新路径' : ''} disabled={!item.dst || done} onclick={startEdit}>
      <code class="to">{item.dst || '保持原位'}</code>
    </button>
  {/if}
  <small>{item.reason}{item.extras.length ? `（配套 ${item.extras.length} 个）` : ''}</small>
</div>

<style>
  .to-btn {
    grid-column: 2;
    background: none;
    border: 0;
    padding: 0;
    margin: 0;
    text-align: left;
    color: inherit;
    cursor: pointer;
    min-width: 0;
    font: inherit;
  }
  .to-btn:disabled {
    cursor: default;
  }
  .to-btn code {
    display: block;
  }
</style>
