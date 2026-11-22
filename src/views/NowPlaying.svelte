<script lang="ts">
  import Cover from '../components/Cover.svelte';
  import Lyrics from '../components/Lyrics.svelte';
  import Icon from '../lib/Icon.svelte';
  import { player } from '../lib/player.svelte';
  import { ui } from '../lib/ui.svelte';
  import { lib } from '../lib/library.svelte';
  import { fmtT } from '../lib/util';
  import { onMount } from 'svelte';

  const c = $derived(player.current);
  let canvas: HTMLCanvasElement;
  let vinyl: HTMLDivElement;

  onMount(() => {
    let raf = 0, ang = 0, last = performance.now(), spin = 0;
    const g = canvas.getContext('2d')!;
    const frame = (now: number) => {
      const dt = Math.min(0.1, (now - last) / 1000);
      last = now;
      // 唱片转速随播放状态缓慢起停
      spin += ((player.playing ? 1 : 0) - spin) * Math.min(1, dt * 2.5);
      ang = (ang + dt * spin * 40) % 360;
      if (vinyl) vinyl.style.transform = `rotate(${ang}deg)`;
      const dpr = devicePixelRatio || 1;
      const w = canvas.clientWidth * dpr, h = canvas.clientHeight * dpr;
      if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
      g.clearRect(0, 0, w, h);
      const n = 72, s = player.spectrum(n / 2), cx = w / 2, cy = h / 2, R = w * 0.335;
      const hue = player.current?.hue ?? 200;
      for (let i = 0; i < n; i++) {
        const v = s[i < n / 2 ? i : n - 1 - i], a = (i / n) * Math.PI * 2 - Math.PI / 2, len = 2 + v * w * 0.1;
        g.strokeStyle = `hsl(${hue + (i < n / 2 ? i : n - i) * 2.2} 90% 72% / ${0.18 + v * 0.6})`;
        g.lineWidth = 3 * dpr;
        g.lineCap = 'round';
        g.beginPath();
        g.moveTo(cx + Math.cos(a) * R, cy + Math.sin(a) * R);
        g.lineTo(cx + Math.cos(a) * (R + len), cy + Math.sin(a) * (R + len));
        g.stroke();
      }
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  });
</script>
<section class="stage on" class:q={ui.queueOpen}>
  <div class="deck" data-drift>
    <canvas id="ringc" class="mp-bgfx" bind:this={canvas}></canvas>
    <div class="vinylwrap"><div class="vinyl" bind:this={vinyl}><Cover tag="i" cls="" cover={c?.cover} name={c?.album ?? ''} hue={c?.hue} /></div></div>
    <Cover tag="div" cls="sleeve" cover={c?.cover} name={c?.album ?? ''} hue={c?.hue} />
    <div class="meta">
      {#if c}
        <b>{c.title}</b><span>{c.artist || '未知艺人'} — {c.album || '未知专辑'}</span>
        <div style="margin-top:10px;display:flex;gap:6px;justify-content:center">
          <button class="mp-chip" class:on={c.favorite} onclick={() => lib.toggleFav(c.id).then(v => (c.favorite = v))}><Icon name="heart" /> {c.favorite ? '已收藏' : '收藏'}</button>
          <button class="mp-chip" onclick={() => ui.open('playlist-add', [c.id])}><Icon name="plus" /> 加入歌单</button>
        </div>
      {:else}
        <b>还没有在播放</b><span>去音乐库选一首，或者把文件拖进窗口</span>
        <div style="margin-top:12px"><button class="mp-btn" onclick={() => (lib.tracks.length ? player.playList(lib.tracks, 0) : ui.open('scan'))}>{lib.tracks.length ? `播放全部 ${lib.tracks.length} 首` : '扫描媒体库'}</button></div>
      {/if}
    </div>
  </div>
  <Lyrics />
  <div class="qpanel">
    <h6>接下来播放 · {player.order.length} 首</h6>
    {#each player.order as qi, p (p + ':' + qi)}
      {@const t = player.queue[qi]}
      {#if t && p >= player.pos - 1}
        <div class="mp-qrow" class:on={p === player.pos} role="button" tabindex="-1" onclick={() => player.jump(p)} onkeydown={() => {}}>
          <Cover tag="i" cls="" cover={t.cover} name={t.album} hue={t.hue} />
          <span style="min-width:0"><b>{t.title}</b><small>{t.artist}</small></span>
          <em>{#if p !== player.pos}<button style="background:none;border:0;color:inherit;padding:0 4px" title="移出队列" onclick={e => { e.stopPropagation(); player.removeAt(p); }}>✕</button>{/if}{fmtT(t.duration)}</em>
        </div>
      {/if}
    {/each}
  </div>
</section>
