<script lang="ts">
  // 正在播放页的动效快捷切换：背景、封面样式、环形频谱
  import Icon from '../../lib/Icon.svelte';
  import { fx, BG_MODES, DECK_MODES } from '../../lib/fx.svelte';
  import { ui } from '../../lib/ui.svelte';

  let open = $state(false);
  let root: HTMLSpanElement;

  function outside(e: PointerEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false;
  }
  function key(e: KeyboardEvent) {
    if (open && e.key === 'Escape') {
      e.stopPropagation();
      open = false;
    }
  }
</script>

<svelte:window onpointerdown={outside} onkeydowncapture={key} />

<span class="fxq" bind:this={root}>
  <button class="mp-chip" class:on={open} title="背景与封面动效" onclick={() => (open = !open)}><Icon name="image" /> 动效</button>
  {#if open}
    <div class="fxq-pop" role="dialog" aria-label="动效">
      <h6>背景</h6>
      <div class="fxq-grid">
        {#each BG_MODES as m (m.id)}
          <button class:on={fx.s.bg === m.id} title={m.desc} onclick={() => fx.set({ bg: m.id })}>{m.name}</button>
        {/each}
      </div>
      <h6>封面</h6>
      <div class="fxq-grid">
        {#each DECK_MODES as m (m.id)}
          <button class:on={fx.s.deck === m.id} title={m.desc} onclick={() => fx.set({ deck: m.id })}>{m.name}</button>
        {/each}
        <button class:on={fx.s.ring} title="封面外圈的频谱" onclick={() => fx.set({ ring: !fx.s.ring })}>环形频谱</button>
      </div>
      {#if ui.oled.black}<p>OLED 纯黑背景已开启，背景动效暂停</p>{/if}
      {#if fx.reduced}<p>系统开启了“减少动态效果”，动效保持静止</p>{/if}
      <button
        class="fxq-more"
        onclick={() => {
          open = false;
          ui.go('settings');
        }}>强度、速度、帧率…</button>
    </div>
  {/if}
</span>

<style>
  .fxq {
    position: relative;
    display: inline-block;
  }
  .fxq-pop {
    position: absolute;
    left: 50%;
    bottom: calc(100% + 8px);
    translate: -50% 0;
    z-index: 600;
    width: 300px;
    padding: 10px 12px 12px;
    border-radius: 14px;
    background: var(--mp-panel);
    backdrop-filter: blur(30px);
    border: 1px solid var(--edge, var(--mp-line));
    box-shadow: var(--mp-shadow);
    text-align: left;
    animation: mp-pop 0.25s cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  h6 {
    margin: 4px 2px 6px;
    font-size: 11px;
    color: var(--mp-sub);
    font-weight: 600;
    letter-spacing: 0.08em;
  }
  .fxq-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 5px;
    margin-bottom: 6px;
  }
  .fxq-grid button {
    border: 1px solid var(--mp-line);
    background: var(--mp-panel2);
    border-radius: 9px;
    padding: 6px 4px;
    font-size: 12.5px;
  }
  .fxq-grid button.on {
    border-color: var(--mp-accent);
    color: var(--mp-accent);
  }
  p {
    margin: 4px 2px;
    font-size: 12px;
    color: var(--mp-sub);
  }
  .fxq-more {
    margin-top: 4px;
    width: 100%;
    background: none;
    border: 0;
    color: var(--mp-accent);
    font-size: 12.5px;
    padding: 6px;
    border-radius: 8px;
  }
  .fxq-more:hover {
    background: var(--mp-panel2);
  }
</style>
