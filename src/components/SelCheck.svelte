<script lang="ts">
  // 多选勾选框：歌曲行的序号位置、卡片 / 剧集缩略图的左上角
  import Icon from '../lib/Icon.svelte';
  let { checked, onclick, card = false }: { checked: boolean; onclick: (e: MouseEvent) => void; card?: boolean } = $props();
</script>

<span class="selck" class:on={checked} class:oncard={card} role="checkbox" aria-checked={checked} aria-label="选择" tabindex="-1"
  onclick={e => { e.stopPropagation(); e.preventDefault(); onclick(e); }}
  ondblclick={e => e.stopPropagation()} onmousedown={e => e.preventDefault()} onkeydown={() => {}}
  >{#if checked}<Icon name="check" />{/if}</span>

<style>
  .selck { display: inline-grid; place-items: center; width: 18px; height: 18px; border-radius: 5px; border: 1.5px solid rgba(255, 255, 255, .55);
    color: #0b0b10; font-size: 12px; line-height: 0; cursor: pointer; box-sizing: border-box; transition: background .12s, border-color .12s, opacity .15s; }
  .selck:hover { border-color: #fff; }
  .selck.on { background: var(--mp-accent); border-color: var(--mp-accent); }
  /* 卡片左上角：悬停或多选状态下显示 */
  .selck.oncard { position: absolute; left: 8px; top: 8px; z-index: 4; width: 22px; height: 22px; border-radius: 6px; background: rgba(0, 0, 0, .5); opacity: 0; font-size: 14px; }
  .selck.oncard.on { background: var(--mp-accent); opacity: 1; }
  :global(.mp-card:hover) .selck.oncard, :global(.ep:hover) .selck.oncard, :global(.multi) .selck.oncard { opacity: 1; }
  /* 勾选框出现时，左上角的角标 / 集数往右让一让 */
  :global(.mp-card:hover .mp-poster .mp-badge), :global(.multi .mp-poster .mp-badge) { left: 38px; }
  :global(.ep:hover .th b), :global(.multi .ep .th b) { left: 40px; }
  :global(.mp-poster .mp-badge), :global(.ep .th b) { transition: left .15s; }
  /* 选中的卡片 */
  :global(.mp-card.picked .mp-poster), :global(.ep.picked .th) { outline: 2px solid var(--mp-accent); outline-offset: 2px; }
</style>
