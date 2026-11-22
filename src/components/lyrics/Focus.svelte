<script lang="ts">
  // 只显示附近几行的歌词：淡入上浮（fade）、打字机（typewriter）。
  // 切行时旧行向上淡出、新行从下方浮入，只用 transform / opacity。
  import { fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import type { LyricLine } from '../../lib/api';
  import { lineUnits, unitProgress, lineProgress } from '../../lib/lyricsfx.svelte';

  let {
    lines,
    k,
    now,
    dur,
    mode,
    pos = 0.42,
    onseek,
  }: {
    lines: LyricLine[];
    k: number;
    now: number;
    dur: number;
    mode: 'fade' | 'typewriter';
    pos?: number;
    onseek?: (t: number) => void;
  } = $props();

  const cur = $derived(lines[k]);
  const units = $derived(k >= 0 ? lineUnits(lines, k, dur) : []);
  const lp = $derived(mode === 'fade' ? lineProgress(units, now) : 0);
  // 打字机：最后一个已经出现的字后面显示光标
  const tip = $derived.by(() => {
    let j = -1;
    for (let i = 0; i < units.length; i++) if (unitProgress(units[i], now) > 0) j = i;
    return j;
  });
  const inY = 34;
  const tIn = { y: inY, duration: 520, easing: cubicOut };
  const tOut = { y: -inY, duration: 420, easing: cubicOut };
</script>

<div class="ly-box ly-focus m-{mode}" data-mp="lyrics">
  <div class="ly-stack" style:top="{pos * 100}%">
    <div class="ly-slot prev">
      {#key k}
        {#if k > 0}
          <div class="mp-ly-line past" role="button" tabindex="-1" in:fly={tIn} out:fly={tOut} onclick={() => onseek?.(lines[k - 1].t)} onkeydown={() => {}}>
            <span>{lines[k - 1].text}</span>
          </div>
        {/if}
      {/key}
    </div>
    <div class="ly-slot cur">
      {#key k}
        {#if cur}
          <div class="mp-ly-line on" in:fly={tIn} out:fly={tOut} style:--lp={lp}>
            {#if mode === 'typewriter'}
              <span class="ly-units">{#each units as u, i}<span class="u" class:hit={i <= tip} class:tip={i === tip}>{u.text}</span>{/each}</span>
            {:else}
              <span>{cur.text}</span>
            {/if}
          </div>
        {/if}
      {/key}
    </div>
    <div class="ly-slot next">
      {#key k}
        {#if mode === 'fade' && lines[k + 1]}
          <div class="mp-ly-line" role="button" tabindex="-1" in:fly={tIn} out:fly={tOut} onclick={() => onseek?.(lines[k + 1].t)} onkeydown={() => {}}>
            <span>{lines[k + 1].text}</span>
          </div>
        {/if}
      {/key}
    </div>
  </div>
</div>
