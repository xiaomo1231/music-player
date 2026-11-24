<script lang="ts">
  // B 站结果的下载选项弹层：下载什么（音视频 / 仅音频 / 仅视频 / 仅元数据）、清晰度、字幕、弹幕、封面
  import { onMount } from 'svelte';
  import { inTauri } from '../../lib/api';
  import { fetchApi } from '../../lib/fetch';
  import { BILI_HEIGHTS, BILI_MODES, hasPicture, hasSavedChoice, hasSubs, saveChoice, type BiliChoice } from '../../lib/bilidl';

  let {
    mk,
    choice = $bindable(),
    confirmText,
    onconfirm,
    onclose,
  }: {
    mk: string;
    choice: BiliChoice;
    confirmText: string;
    onconfirm: (c: BiliChoice) => void;
    onclose: () => void;
  } = $props();

  let el: HTMLDivElement;

  onMount(() => {
    // 第一次用时字幕、弹幕跟设置走
    if (inTauri && !hasSavedChoice(mk)) {
      fetchApi
        .getSettings()
        .then(s => (choice = { ...choice, subs: s.bili_subs, danmaku: s.bili_danmaku }))
        .catch(() => {});
    }
    const away = (e: PointerEvent) => {
      if (el && !el.contains(e.target as Node) && !(e.target as HTMLElement)?.closest?.('[data-bili-dl]')) onclose();
    };
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && onclose();
    document.addEventListener('pointerdown', away, true);
    document.addEventListener('keydown', esc);
    return () => {
      document.removeEventListener('pointerdown', away, true);
      document.removeEventListener('keydown', esc);
    };
  });

  const hint = $derived(BILI_MODES.find(m => m.key === choice.mode)?.hint ?? '');

  function go() {
    saveChoice(mk, choice);
    onconfirm({ ...choice });
  }
</script>

<div class="bdm" bind:this={el} role="dialog" aria-label="下载选项">
  <div class="row-lbl">下载内容</div>
  <div class="seg">
    {#each BILI_MODES as m (m.key)}
      <button class="mp-chip" class:on={choice.mode === m.key} onclick={() => (choice = { ...choice, mode: m.key })}>{m.label}</button>
    {/each}
  </div>
  <div class="hint">{hint}</div>

  {#if hasPicture(choice.mode)}
    <div class="row-lbl">清晰度</div>
    <div class="seg">
      {#each BILI_HEIGHTS as [k, l] (k)}
        <button class="mp-chip" class:on={choice.height === k} onclick={() => (choice = { ...choice, height: k })}>{l}</button>
      {/each}
    </div>
  {/if}

  <div class="row-lbl">附带</div>
  <div class="checks">
    {#if hasSubs(choice.mode)}
      <label><input type="checkbox" bind:checked={choice.subs} /> 字幕</label>
      <label><input type="checkbox" bind:checked={choice.danmaku} /> 弹幕</label>
    {/if}
    {#if choice.mode === 'meta'}
      <span class="fixed">信息、简介、封面</span>
    {:else}
      <label><input type="checkbox" bind:checked={choice.cover} /> 封面图</label>
    {/if}
  </div>

  <div class="acts">
    <button class="mp-btn ghost sm" onclick={onclose}>取消</button>
    <button class="mp-btn sm" onclick={go}>{confirmText}</button>
  </div>
</div>

<style>
  .bdm {
    position: absolute;
    z-index: 30;
    top: calc(100% + 6px);
    left: 0;
    width: 300px;
    max-width: calc(100vw - 32px);
    padding: 10px 12px;
    border-radius: 12px;
    background: var(--mp-panel);
    backdrop-filter: blur(var(--mp-blur, 24px));
    border: 1px solid rgba(255, 255, 255, 0.12);
    box-shadow: var(--mp-shadow);
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12.5px;
  }
  .row-lbl {
    color: var(--mp-sub);
    font-size: 11.5px;
    margin-top: 2px;
  }
  .seg {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .seg .mp-chip.on {
    background: color-mix(in srgb, var(--mp-accent) 30%, transparent);
    border-color: var(--mp-accent);
  }
  .hint {
    color: var(--mp-sub);
    font-size: 11.5px;
  }
  .checks {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    align-items: center;
  }
  .checks label {
    display: inline-flex;
    gap: 4px;
    align-items: center;
    cursor: pointer;
  }
  .fixed {
    color: var(--mp-sub);
  }
  .acts {
    display: flex;
    gap: 6px;
    justify-content: flex-end;
    margin-top: 4px;
  }
  .sm {
    padding: 4px 12px;
    font-size: 12px;
  }
</style>
