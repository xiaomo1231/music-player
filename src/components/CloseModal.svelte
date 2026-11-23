<script lang="ts">
  // 点窗口关闭按钮时：选最小化到托盘还是退出，可记住选择；有下载在进行时提醒
  import Modal from './Modal.svelte';
  import { ui, toast } from '../lib/ui.svelte';
  import { closeChoose } from '../lib/shell.svelte';

  const info = $derived(ui.closeAsk);
  let pick = $state<'tray' | 'quit'>('tray');
  let remember = $state(false);
  let busy = $state(false);

  const cancel = () => (ui.closeAsk = null);
  async function go(action: 'tray' | 'quit') {
    if (busy) return;
    busy = true;
    try {
      await closeChoose(action, info?.ask ? remember : false);
      ui.closeAsk = null;
      if (action === 'tray' && remember && info?.ask) toast('已记住：关闭时最小化到托盘，可在设置里修改');
    } catch (e) {
      toast(String(e));
    } finally {
      busy = false;
    }
  }
</script>

{#if info}
  <Modal title={info.ask ? '关闭极光播放器' : '退出极光播放器'} onclose={cancel}>
    {#if info.ask}
      <label class="mp-opt choice">
        <div>最小化到托盘<small>窗口隐藏，音乐和下载继续；单击托盘图标重新打开</small></div>
        <input type="radio" name="close-pick" value="tray" bind:group={pick} />
      </label>
      <label class="mp-opt choice">
        <div>退出程序<small>停止播放并完全关闭</small></div>
        <input type="radio" name="close-pick" value="quit" bind:group={pick} />
      </label>
    {/if}
    {#if info.downloads > 0 && (!info.ask || pick === 'quit')}
      <p class="mp-desc warn">还有 {info.downloads} 个下载在进行，退出后会暂停，下次打开可以在“自动获取”页继续。</p>
    {/if}
    {#if info.ask}
      <label class="remember"><input type="checkbox" bind:checked={remember} /> 记住我的选择，不再询问（可在设置 → 外观与桌面里修改）</label>
    {/if}
    <div class="rowbtns">
      {#if info.ask}
        <button class="mp-btn" disabled={busy} onclick={() => go(pick)}>确定</button>
      {:else}
        <button class="mp-btn" disabled={busy} onclick={() => go('tray')}>最小化到托盘</button>
        <button class="mp-btn ghost" disabled={busy} onclick={() => go('quit')}>仍然退出</button>
      {/if}
      <button class="mp-btn ghost" disabled={busy} onclick={cancel}>取消</button>
    </div>
  </Modal>
{/if}

<style>
  .choice { cursor: pointer; }
  .warn { color: var(--warn, #e0a030); }
  .remember { display: flex; align-items: center; gap: 6px; margin: 12px 0 4px; font-size: 13px; cursor: pointer; }
</style>
