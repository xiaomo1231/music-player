<script lang="ts">
  // 全局的“⋯”菜单 + 改名 / 确认对话框（由 lib/items.svelte.ts 控制）
  import Modal from './Modal.svelte';
  import { items, RESET, undoRemove } from '../lib/items.svelte';
  let el: HTMLDivElement | undefined = $state();
  let pos = $state({ left: 0, top: 0 });
  let input: HTMLInputElement | undefined = $state();

  $effect(() => {
    const m = items.menu;
    if (!m || !el) return;
    pos = { left: Math.max(8, Math.min(m.x - el.offsetWidth, innerWidth - el.offsetWidth - 8)), top: Math.max(8, Math.min(m.y, innerHeight - el.offsetHeight - 8)) };
  });
  $effect(() => {
    if (items.dialog?.type === 'prompt') setTimeout(() => { input?.focus(); input?.select(); }, 60);
  });

  function close() {
    items.menu = null;
  }
  function finish(v: unknown) {
    const d = items.dialog;
    items.dialog = null;
    if (!d) return;
    if (d.type === 'prompt') d.resolve(v as string | null | typeof RESET);
    else d.resolve(!!v);
  }
</script>

<svelte:window
  onpointerdown={e => items.menu && !el?.contains(e.target as Node) && close()}
  onkeydown={e => e.key === 'Escape' && items.menu && close()}
  onblur={close}
  onresize={close} />

{#if items.menu}
  <div class="menu open itemmenu" bind:this={el} style:left="{pos.left}px" style:top="{pos.top}px" role="menu">
    <h6>{items.menu.title}</h6>
    {#each items.menu.entries as en}
      {#if en === 'sep'}<hr />{:else}
        <button class:danger={en.danger} role="menuitem" onclick={() => { close(); en.run(); }}>{en.label}</button>
      {/if}
    {/each}
  </div>
{/if}

{#if items.dialog}
  {@const d = items.dialog}
  <Modal title={d.title} onclose={() => finish(null)}>
    {#if d.type === 'prompt'}
      {#if d.desc}<p class="mp-desc">{d.desc}</p>{/if}
      <input class="tin" style="width:100%" bind:this={input} bind:value={d.value} placeholder={d.placeholder} maxlength="120"
        onkeydown={e => e.key === 'Enter' && finish(d.value)} />
      <div class="rowbtns">
        <button class="mp-btn" onclick={() => finish(d.value)}>确定</button>
        {#if d.reset}<button class="mp-btn ghost" onclick={() => finish(RESET)}>{d.reset}</button>{/if}
        <button class="mp-btn ghost" onclick={() => finish(null)}>取消</button>
      </div>
    {:else}
      <p class="mp-desc">{d.desc}</p>
      {#if d.list?.length}
        <div class="filelist selectable">{#each d.list as f}<code>{f}</code>{/each}</div>
      {/if}
      <div class="rowbtns">
        <button class="mp-btn" class:danger={d.danger} onclick={() => finish(true)}>{d.ok}</button>
        <button class="mp-btn ghost" onclick={() => finish(false)}>取消</button>
      </div>
    {/if}
  </Modal>
{/if}

{#if items.undo}
  <div class="undobar" role="status">
    <span>{items.undo.msg}</span>
    <button class="mp-btn" onclick={undoRemove}>撤销</button>
    <button class="x" title="关闭" aria-label="关闭" onclick={() => (items.undo = null)}>×</button>
  </div>
{/if}

<style>
  .undobar { position: fixed; left: 50%; bottom: 96px; translate: -50% 0; z-index: 1003; display: flex; align-items: center; gap: 12px; max-width: calc(100vw - 32px);
    background: var(--mp-panel); border: 1px solid var(--edge); backdrop-filter: blur(30px); box-shadow: var(--mp-shadow); padding: 8px 10px 8px 18px; border-radius: 999px; font-size: 13px; }
  .undobar span { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .undobar .mp-btn { padding: 5px 14px; }
  .undobar .x { background: none; border: 0; color: var(--mp-sub); font-size: 18px; line-height: 1; padding: 2px 6px; border-radius: 6px; }
  .undobar .x:hover { color: var(--mp-text); background: rgba(255, 255, 255, .1); }
  .itemmenu { z-index: 960; }
  .itemmenu hr { border: 0; border-top: 1px solid var(--edge); margin: 4px 6px; }
  .itemmenu button.danger { color: #ff8a8a; }
  .filelist { max-height: 220px; overflow: auto; display: grid; gap: 3px; padding: 8px; border-radius: 10px; background: rgba(255, 255, 255, .05); }
  .filelist code { font-family: Consolas, monospace; font-size: 11.5px; color: var(--mp-sub); word-break: break-all; }
  .mp-btn.danger { background: #e5484d; }
</style>
