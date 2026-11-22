<script lang="ts">
  import Modal from './Modal.svelte';
  import { updater } from '../lib/updater.svelte';
  import { engine } from '../lib/engine.svelte';
  import { fmtSize } from '../lib/util';

  const pct = $derived(updater.total ? Math.min(100, Math.round((updater.downloaded / updater.total) * 100)) : 0);
  const working = $derived(updater.phase === 'downloading' || updater.phase === 'installing');
</script>

<Modal title="发现新版本" onclose={() => updater.later()}>
  <div class="upd">
    <p class="ver">
      <b>{updater.version}</b>
      <span>当前 {engine.version || '—'}</span>
    </p>
    {#if updater.notes}
      <div class="notes">{updater.notes}</div>
    {:else}
      <p class="sub">这个版本没有填写更新说明。</p>
    {/if}

    {#if updater.phase === 'downloading'}
      <div class="bar"><i style="width:{pct}%"></i></div>
      <p class="sub">
        正在下载 {fmtSize(updater.downloaded)}{updater.total ? ` / ${fmtSize(updater.total)}（${pct}%）` : ''}
      </p>
    {:else if updater.phase === 'installing'}
      <p class="sub">下载完成，正在安装，程序会自动关闭并重新打开…</p>
    {:else if updater.phase === 'error'}
      <p class="err">更新失败：{updater.error}</p>
    {:else}
      <p class="sub">更新会下载安装包并覆盖安装，媒体库和设置都会保留。安装时程序会自动关闭再重新打开。</p>
    {/if}

    <div class="btns">
      {#if !working}
        <button class="mp-btn ghost" onclick={() => updater.skip()}>跳过这个版本</button>
        <button class="mp-btn ghost" onclick={() => updater.later()}>以后再说</button>
      {/if}
      <button class="mp-btn" disabled={working} onclick={() => updater.install()}>
        {working ? '更新中…' : updater.phase === 'error' ? '重试' : '立即更新'}
      </button>
    </div>
  </div>
</Modal>

<style>
  .upd {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 380px;
  }
  .ver {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin: 0;
  }
  .ver b {
    font-size: 20px;
  }
  .ver span,
  .sub {
    color: var(--mp-sub);
    font-size: 12.5px;
    margin: 0;
    line-height: 1.6;
  }
  .notes {
    white-space: pre-wrap;
    max-height: 260px;
    overflow: auto;
    font-size: 13px;
    line-height: 1.7;
    padding: 10px 12px;
    border-radius: 10px;
    background: rgba(255, 255, 255, 0.05);
  }
  .err {
    color: #ff8a8a;
    font-size: 12.5px;
    margin: 0;
    word-break: break-all;
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.1);
    overflow: hidden;
  }
  .bar i {
    display: block;
    height: 100%;
    background: var(--mp-accent, #7cc4ff);
    transition: width 0.2s;
  }
  .btns {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
</style>
