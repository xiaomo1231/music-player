<script lang="ts">
  // 歌词区右上角的快捷切换：悬停歌词区时出现，点开选样式、字号、对齐和模糊，选择即时生效并保存
  import { lyricsfx, LYRIC_STYLES, LYRIC_ALIGNS } from '../../lib/lyricsfx.svelte';

  let open = $state(false);
  let root = $state<HTMLDivElement>();
  const cfg = $derived(lyricsfx.cfg);

  $effect(() => {
    if (!open) return;
    const down = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node)) open = false;
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        open = false;
      }
    };
    window.addEventListener('pointerdown', down, true);
    window.addEventListener('keydown', key, true);
    return () => {
      window.removeEventListener('pointerdown', down, true);
      window.removeEventListener('keydown', key, true);
    };
  });
</script>

<div class="ly-fx" class:open bind:this={root}>
  <button class="ly-fxbtn" title="歌词样式" aria-label="歌词样式" aria-expanded={open} onclick={() => (open = !open)}>Aa</button>
  {#if open}
    <div class="ly-fxpop" role="dialog" aria-label="歌词样式">
      <h6>歌词样式</h6>
      <div class="ly-fxgrid">
        {#each LYRIC_STYLES as s}
          <button class:on={cfg.style === s.id} title={s.desc} onclick={() => lyricsfx.set({ style: s.id })}>{s.name}</button>
        {/each}
      </div>
      <div class="ly-fxrow">
        <span>字号</span>
        <input type="range" min="0.7" max="1.6" step="0.05" value={cfg.size} aria-label="歌词字号" oninput={e => lyricsfx.set({ size: +(e.currentTarget as HTMLInputElement).value })} />
        <em>{Math.round(cfg.size * 100)}%</em>
      </div>
      <div class="ly-fxrow">
        <span>对齐</span>
        <div class="seg">
          {#each LYRIC_ALIGNS as a}
            <button class:on={cfg.align === a.id} onclick={() => lyricsfx.set({ align: a.id })}>{a.name}</button>
          {/each}
        </div>
      </div>
      <label class="ly-fxrow">
        <span>模糊非当前行</span>
        <input type="checkbox" class="mp-sw" checked={cfg.blur} onchange={e => lyricsfx.set({ blur: (e.currentTarget as HTMLInputElement).checked })} />
      </label>
    </div>
  {/if}
</div>
