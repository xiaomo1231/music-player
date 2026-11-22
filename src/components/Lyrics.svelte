<script lang="ts">
  // 滚动歌词：当前行居中高亮，逐字 / 逐行进度用渐变填充
  import { player } from '../lib/player.svelte';
  let { cls = 'lyr', pos = 0.42 }: { cls?: string; pos?: number } = $props();
  let box: HTMLDivElement;
  let inner: HTMLDivElement;
  let k = $state(-1);
  let lp = $state(0);
  const timed = $derived(player.lyrics.length > 0 && player.lyrics[0].t >= 0);

  $effect(() => {
    let raf = 0;
    const tick = () => {
      const nk = player.lyricIndex();
      if (nk !== k) k = nk;
      lp = player.lyricProgress(nk);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });

  $effect(() => {
    void player.lyrics; void k;
    if (!inner || !box) return;
    const el = inner.children[Math.max(k, 0)] as HTMLElement | undefined;
    if (!el) { inner.style.transform = ''; return; }
    inner.style.transform = `translateY(${box.clientHeight * pos - (el.offsetTop + el.offsetHeight / 2)}px)`;
  });
</script>
<div class={cls} data-mp="lyrics" data-drift bind:this={box}>
  <div class="mp-ly-inner" bind:this={inner} style="position:relative">
    {#if !player.lyrics.length}
      <div class="mp-ly-empty">{player.current ? '纯音乐 / 暂无歌词' : '还没有在播放'}<br /><small style="font-size:.5em;font-weight:400">把同名 .lrc 文件放在歌曲旁边即可显示歌词</small></div>
    {:else}
      {#each player.lyrics as l, i}
        <div class="mp-ly-line" class:on={i === k} class:past={i < k} class:plain={!timed} style:--d={timed ? Math.min(Math.abs(i - k), 6) : 0} style:--lp={i === k ? lp : 0}
          role="button" tabindex="-1" onclick={() => { if (timed) { player.seek(l.t); if (!player.playing) player.play(); } }} onkeydown={() => {}}>
          <span>{l.text}</span>
        </div>
      {/each}
    {/if}
  </div>
</div>
