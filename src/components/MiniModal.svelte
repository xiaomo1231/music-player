<script lang="ts">
  import Modal from './Modal.svelte';
  import { ui } from '../lib/ui.svelte';
  const m = ui.miniCfg;
  const save = () => ui.saveMini();
  const BGS: [typeof m.bg, string][] = [['theme', '跟随主题'], ['dark', '深灰'], ['black', '纯黑'], ['light', '浅色']];
  const ACCENTS = ['', '#5ee6e6', '#7aa2ff', '#b38cff', '#ff7ab6', '#ff9f5a', '#ffd24d', '#6fe08a'];
  async function enter() {
    ui.close();
    ui.setMini(true);
  }
</script>

<Modal title="迷你播放器" onclose={() => ui.close()}>
  <p class="mp-desc">置顶小窗的外观，改动会立即生效并自动保存。小窗上的齿轮按钮可以随时回到这里。</p>
  <div class="mp-opt"><div>背景<small>小窗的底色</small></div>
    <select bind:value={m.bg} onchange={save}>{#each BGS as [v, l]}<option value={v}>{l}</option>{/each}</select></div>
  <div class="mp-opt"><div>强调色<small>歌词、进度条的颜色；第一个是跟随主题</small></div>
    <div class="mp-swatches">
      {#each ACCENTS as a}
        <button class:on={m.accent === a} class:auto={!a} style:--sw={a || 'var(--mp-accent)'} title={a || '跟随主题'} aria-label={a || '跟随主题'} onclick={() => { m.accent = a; save(); }}></button>
      {/each}
    </div></div>
  <div class="mp-opt"><div>第二行显示<small>当前一句歌词，或歌手名</small></div>
    <select bind:value={m.line} onchange={save}><option value="lyric">歌词</option><option value="artist">歌手</option></select></div>
  <label class="mp-opt"><div>封面转动<small>播放时封面像唱片一样慢慢旋转</small></div><input type="checkbox" class="mp-sw" bind:checked={m.spin} onchange={save} /></label>
  <div class="mp-opt"><div>打开迷你播放器<small>快捷键 Ctrl + M 也可以切换</small></div><button class="mp-btn ghost" onclick={enter}>进入</button></div>
</Modal>

<style>
  .mp-swatches { display: flex; gap: 6px; flex-wrap: wrap; justify-content: flex-end }
  .mp-swatches button { width: 22px; height: 22px; border-radius: 50%; border: 2px solid transparent; background: var(--sw); padding: 0; outline-offset: 2px }
  .mp-swatches button.auto { background: conic-gradient(from 0deg, #ff7ab6, #ffd24d, #6fe08a, #5ee6e6, #7aa2ff, #b38cff, #ff7ab6) }
  .mp-swatches button.on { border-color: var(--mp-text) }
</style>
