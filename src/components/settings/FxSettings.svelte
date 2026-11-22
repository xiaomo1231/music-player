<script lang="ts">
  // 设置页“外观与动效”分组：背景动效和封面区动效，改动即时生效并保存
  import { fx, BG_MODES, DECK_MODES, FPS_MODES, FX_DEFAULT } from '../../lib/fx.svelte';
  import { ui } from '../../lib/ui.svelte';
  import FxPreview from '../fx/FxPreview.svelte';

  const num = (e: Event) => Number((e.currentTarget as HTMLInputElement).value);
  const bgDesc = $derived(BG_MODES.find(m => m.id === fx.s.bg)?.desc ?? '');
  const deckDesc = $derived(DECK_MODES.find(m => m.id === fx.s.deck)?.desc ?? '');
</script>

<div class="card fxs">
  <h5>外观与动效</h5>
  <div class="fxs-body">
    <div class="fxs-prev">
      <FxPreview />
      <small>预览 · {bgDesc}</small>
      {#if ui.oled.black}<small class="warn">OLED 防烧屏的“纯黑背景”已开启，背景保持纯黑、不绘制动效</small>{/if}
      {#if fx.reduced}<small class="warn">系统开启了“减少动态效果”，背景和封面保持静止</small>{/if}
    </div>
    <div class="fxs-opts">
      <div class="mp-opt col">
        <div>背景</div>
        <div class="seg">
          {#each BG_MODES as m (m.id)}
            <button class:on={fx.s.bg === m.id} title={m.desc} onclick={() => fx.set({ bg: m.id })}>{m.name}</button>
          {/each}
        </div>
      </div>
      <div class="mp-opt">
        <div>强度 {fx.s.intensity}<small>光斑亮度、律动幅度</small></div>
        <input type="range" min="0" max="100" value={fx.s.intensity} oninput={e => fx.set({ intensity: num(e) })} aria-label="动效强度" />
      </div>
      <div class="mp-opt">
        <div>速度 {fx.s.speed}<small>漂移、旋转快慢</small></div>
        <input type="range" min="0" max="100" value={fx.s.speed} oninput={e => fx.set({ speed: num(e) })} aria-label="动效速度" />
      </div>
      <label class="mp-opt">
        <div>跟随封面取色<small>颜色取自当前封面主色，换歌时平滑过渡</small></div>
        <input type="checkbox" class="mp-sw" checked={fx.s.followCover} onchange={e => fx.set({ followCover: (e.currentTarget as HTMLInputElement).checked })} />
      </label>
      {#if !fx.s.followCover}
        <div class="mp-opt">
          <div>固定色调 <i class="sw" style:background={`hsl(${fx.s.hue} 85% 55%)`}></i></div>
          <input type="range" min="0" max="360" value={fx.s.hue} oninput={e => fx.set({ hue: num(e) })} aria-label="固定色调" />
        </div>
      {/if}
      <div class="mp-opt">
        <div>帧率<small>省电档约 20 帧、降低分辨率；窗口最小化或隐藏时动效自动暂停</small></div>
        <div class="seg">
          {#each FPS_MODES as m (m.id)}
            <button class:on={fx.s.fps === m.id} onclick={() => fx.set({ fps: m.id })}>{m.name}</button>
          {/each}
        </div>
      </div>
      <div class="mp-opt">
        <div>封面<small>{deckDesc}</small></div>
        <div class="seg">
          {#each DECK_MODES as m (m.id)}
            <button class:on={fx.s.deck === m.id} title={m.desc} onclick={() => fx.set({ deck: m.id })}>{m.name}</button>
          {/each}
        </div>
      </div>
      <label class="mp-opt">
        <div>环形频谱<small>封面外圈的频谱跟随音乐跳动</small></div>
        <input type="checkbox" class="mp-sw" checked={fx.s.ring} onchange={e => fx.set({ ring: (e.currentTarget as HTMLInputElement).checked })} />
      </label>
      <div class="rowbtns">
        <button class="mp-btn ghost" onclick={() => fx.set({ ...FX_DEFAULT })}>恢复默认</button>
      </div>
    </div>
  </div>
</div>

<style>
  .fxs {
    grid-column: 1 / -1;
  }
  .fxs-opts .mp-opt > div:first-child {
    min-width: 8em;
  }
  .fxs-body {
    display: grid;
    grid-template-columns: minmax(220px, 300px) minmax(0, 1fr);
    gap: 18px;
    align-items: start;
  }
  .fxs-prev {
    display: flex;
    flex-direction: column;
    gap: 6px;
    position: sticky;
    top: 0;
  }
  .fxs-prev small {
    color: var(--mp-sub);
    font-size: 12px;
    line-height: 1.5;
  }
  .fxs-prev .warn {
    color: var(--mp-accent);
  }
  .mp-opt.col {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }
  .mp-opt:first-child {
    border-top: 0;
  }
  input[type='range'] {
    flex: 0 1 220px;
    accent-color: var(--mp-accent);
    min-width: 120px;
  }
  .sw {
    display: inline-block;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    vertical-align: -1px;
    margin-left: 4px;
  }
  .rowbtns {
    display: flex;
    justify-content: flex-end;
    padding-top: 8px;
  }
  @media (max-width: 860px) {
    .fxs-body {
      grid-template-columns: 1fr;
    }
    .fxs-prev {
      position: static;
    }
  }
</style>
