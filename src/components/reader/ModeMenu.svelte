<script lang="ts">
  // 小说阅读器“翻页方式”弹出面板：上下滚动 / 左右翻页 / 自动滚动，翻页动画，自动滚动速度。
  // 顶栏可以横向滚动（会裁掉子元素），所以面板用 fixed 定位，按按钮的位置摆放
  import { NOVEL_MODES, PAGE_ANIMS, SPEED_MIN, SPEED_MAX, type NovelMode, type PageAnim } from '../../lib/readerprefs';
  import { FLOW_ICON } from '../comic/icons';

  let {
    mode,
    anim,
    speed,
    x,
    y,
    onmode,
    onanim,
    onspeed,
    onclose,
  }: {
    mode: NovelMode;
    anim: PageAnim;
    speed: number;
    x: number;
    y: number;
    onmode: (m: NovelMode) => void;
    onanim: (a: PageAnim) => void;
    onspeed: (v: number) => void;
    onclose: () => void;
  } = $props();

  function outside(e: PointerEvent) {
    const t = e.target as HTMLElement;
    if (!t.closest('.nm-pop') && !t.closest('.nm-btn')) onclose();
  }
</script>

<svelte:window onpointerdown={outside} />

<div class="nm-pop" style="left:{Math.max(8, x)}px;top:{y}px">
  <h6>翻页方式</h6>
  {#each NOVEL_MODES as m (m.id)}
    <button type="button" class="nm-opt" class:on={mode === m.id} onclick={() => onmode(m.id)}>
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">{@html FLOW_ICON[m.id] ?? ''}</svg>
      <span><b>{m.name}</b><small>{m.desc}</small></span>
    </button>
  {/each}
  <h6>翻页动画<small>{mode === 'page' ? '' : '（左右翻页时有效）'}</small></h6>
  <div class="nm-seg">
    {#each PAGE_ANIMS as a (a.id)}
      <button type="button" class:on={anim === a.id} disabled={mode !== 'page'} onclick={() => onanim(a.id)}>{a.name}</button>
    {/each}
  </div>
  <h6>自动滚动速度 {speed}<small>↑ / ↓ 调整</small></h6>
  <input
    type="range"
    min={SPEED_MIN}
    max={SPEED_MAX}
    step="5"
    value={speed}
    aria-label="自动滚动速度"
    oninput={e => onspeed(Number((e.currentTarget as HTMLInputElement).value))}
  />
</div>

<style>
  .nm-pop {
    position: fixed;
    z-index: 30;
    width: 300px;
    background: #1b1b22;
    color: #eee;
    border: 1px solid #ffffff22;
    border-radius: 12px;
    padding: 10px;
    box-shadow: 0 14px 40px #000a;
    display: grid;
    gap: 6px;
    font-family: var(--mp-font);
    font-size: 13px;
  }
  h6 {
    margin: 6px 2px 0;
    font-size: 12px;
    font-weight: 600;
    color: #bbb;
  }
  h6 small {
    margin-left: 6px;
    font-weight: 400;
    color: #888;
  }
  .nm-opt {
    display: flex;
    gap: 10px;
    align-items: center;
    text-align: left;
    background: none;
    border: 1px solid transparent;
    border-radius: 9px;
    padding: 7px 9px;
    color: #eee;
    cursor: pointer;
    opacity: 1;
  }
  .nm-opt:hover {
    background: #ffffff10;
  }
  .nm-opt.on {
    background: #ffffff1c;
    border-color: #ffffff30;
  }
  .nm-opt svg {
    width: 24px;
    height: 24px;
    flex: none;
  }
  .nm-opt b {
    display: block;
    font-size: 13px;
  }
  .nm-opt small {
    display: block;
    font-size: 11.5px;
    color: #999;
  }
  .nm-seg {
    display: inline-flex;
    background: #ffffff10;
    border: 1px solid #ffffff1c;
    border-radius: 9px;
    padding: 2px;
    gap: 2px;
  }
  .nm-seg button {
    background: none;
    border: 0;
    border-radius: 7px;
    padding: 4px 10px;
    font-size: 12.5px;
    color: #eee;
    cursor: pointer;
  }
  .nm-seg button.on {
    background: #fff;
    color: #111;
  }
  .nm-seg button:disabled {
    opacity: 0.35;
    cursor: default;
  }
  input[type='range'] {
    accent-color: #b0864a;
  }
</style>
