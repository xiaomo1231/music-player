<script lang="ts">
  import Cover from '../components/Cover.svelte';
  import Lyrics from '../components/Lyrics.svelte';
  import Icon from '../lib/Icon.svelte';
  import { player } from '../lib/player.svelte';
  import { ui } from '../lib/ui.svelte';
  import { lib } from '../lib/library.svelte';
  import { fmtT } from '../lib/util';
  import { onMount } from 'svelte';
  import { fx, fxLoop, fxFrameMs } from '../lib/fx.svelte';
  import { coverHue } from '../components/fx/driver';
  import FxQuick from '../components/fx/FxQuick.svelte';

  const c = $derived(player.current);
  let canvas: HTMLCanvasElement;
  let vinyl: HTMLDivElement;
  let deck: HTMLDivElement;

  // 封面区动效：黑胶旋转 / 封面呼吸 / 环形频谱。设置见 lib/fx.svelte.ts
  const deckMode = $derived(fx.s.deck);
  const run = $derived(fx.visible && !ui.mini && ui.videoId == null && ui.bookId == null && ui.comicId == null);
  let ang = 0, spin = 0, rot = 0, bt = 0;
  let cw = 0, ch = 0;
  /** 静止状态（减少动态效果等）下改尺寸后重画一帧 */
  let redrawStill: (() => void) | null = null;

  onMount(() => {
    const ro = new ResizeObserver(() => {
      cw = canvas.clientWidth;
      ch = canvas.clientHeight;
      redrawStill?.();
    });
    ro.observe(canvas);
    return () => ro.disconnect();
  });

  function drawRing(g: CanvasRenderingContext2D, dt: number, still: boolean) {
    const dpr = Math.min(devicePixelRatio || 1, fx.eco ? 1 : 1.5);
    const w = Math.round(cw * dpr), h = Math.round(ch * dpr);
    if (!w || !h) return;
    if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
    g.clearRect(0, 0, w, h);
    if (!fx.s.ring) return;
    const n = 72, half = n / 2;
    const s = still ? new Array(half).fill(0.05) : player.fxBands(half);
    const k = fx.intensityMul;
    const cx = w / 2, cy = h / 2;
    const sq = deckMode !== 'vinyl';
    const R = w * 0.335, maxL = w * (sq ? 0.07 : 0.09);
    // 方形封面：刻度沿封面外沿排开（画布是封面的 1.68 倍），底边短一些，免得压住歌名
    const S = w / 1.68 / 2 + 12 * dpr;
    if (!still) rot = (rot + dt * (0.04 + 0.16 * spin) * fx.speedMul) % (Math.PI * 2);
    const hue = fx.s.followCover ? coverHue() : fx.s.hue;
    g.lineWidth = 3 * dpr;
    g.lineCap = 'round';
    for (let i = 0; i < n; i++) {
      const j = i < half ? i : n - 1 - i;
      const v = Math.min(1, s[j] * (0.75 + 0.45 * k));
      const a = (i / n) * Math.PI * 2 - Math.PI / 2 + (sq ? 0 : rot);
      const ca = Math.cos(a), sa = Math.sin(a);
      let x0: number, y0: number, nx: number, ny: number, len = 2 * dpr + v * maxL;
      if (sq) {
        const m = Math.max(Math.abs(ca), Math.abs(sa));
        x0 = cx + (ca / m) * S;
        y0 = cy + (sa / m) * S;
        if (Math.abs(ca) > Math.abs(sa)) { nx = Math.sign(ca); ny = 0; } else { nx = 0; ny = Math.sign(sa); }
        if (ny > 0) len = 2 * dpr + v * maxL * 0.3;
      } else {
        x0 = cx + ca * R;
        y0 = cy + sa * R;
        if (sa > 0) len = 2 * dpr + v * maxL * (1 - 0.75 * sa * sa); // 下方压短，免得压住歌名
        nx = ca;
        ny = sa;
      }
      g.strokeStyle = `hsl(${hue + j * 2.2} 90% 72% / ${Math.min(1, 0.22 + v * 0.75).toFixed(3)})`;
      g.beginPath();
      g.moveTo(x0, y0);
      g.lineTo(x0 + nx * len, y0 + ny * len);
      g.stroke();
    }
  }

  $effect(() => {
    if (!run) return;
    const mode = deckMode, ring = fx.s.ring, reduced = fx.reduced;
    void [fx.s.intensity, fx.s.hue, fx.s.followCover, player.current?.id];
    const g = canvas.getContext('2d')!;
    const sleeve = deck.querySelector<HTMLElement>('.sleeve');
    if (sleeve) sleeve.style.scale = '';
    // 减少动态效果：唱片不转、不呼吸，频谱画一帧静止的
    if (reduced || (mode === 'still' && !ring)) {
      if (vinyl) vinyl.style.transform = `rotate(${ang}deg)`;
      redrawStill = () => drawRing(g, 0, true);
      redrawStill();
      return () => (redrawStill = null);
    }
    return fxLoop((_now, dt) => {
      // 唱片转速随播放状态缓慢起停
      spin += ((player.playing ? 1 : 0) - spin) * Math.min(1, dt * (player.playing ? 2.5 : 0.9));
      if (mode === 'vinyl') {
        ang = (ang + dt * spin * 40) % 360;
        if (vinyl) vinyl.style.transform = `rotate(${ang}deg)`;
      } else if (mode === 'breathe' && sleeve) {
        bt += dt * fx.speedMul;
        const lv = player.levels();
        const sc = 1 + (0.006 + 0.01 * spin) * Math.sin(bt * 1.15) + 0.03 * lv.bass * spin * fx.intensityMul;
        sleeve.style.scale = sc.toFixed(4);
      }
      if (ring) drawRing(g, dt, false);
      else if (canvas.width) g.clearRect(0, 0, canvas.width, canvas.height);
    }, () => fxFrameMs(ui.dimmed, ui.idle && !player.playing));
  });
</script>
<section class="stage on" class:q={ui.queueOpen}>
  <div class="deck" class:sq={deckMode !== 'vinyl'} class:breathe={deckMode === 'breathe'} data-drift bind:this={deck}>
    <canvas id="ringc" bind:this={canvas}></canvas>
    <div class="vinylwrap"><div class="vinyl" bind:this={vinyl}><Cover tag="i" cls="" cover={c?.cover} name={c?.album ?? ''} hue={c?.hue} /></div></div>
    <Cover tag="div" cls="sleeve" cover={c?.cover} name={c?.album ?? ''} hue={c?.hue} />
    <div class="meta">
      {#if c}
        <b>{c.title}</b><span>{c.artist || '未知艺人'} — {c.album || '未知专辑'}</span>
        <div style="margin-top:10px;display:flex;gap:6px;justify-content:center">
          <button class="mp-chip" class:on={c.favorite} onclick={() => lib.toggleFav(c.id).then(v => (c.favorite = v))}><Icon name="heart" /> {c.favorite ? '已收藏' : '收藏'}</button>
          <button class="mp-chip" onclick={() => ui.open('playlist-add', [c.id])}><Icon name="plus" /> 加入歌单</button>
          <FxQuick />
        </div>
      {:else}
        <b>还没有在播放</b><span>去音乐库选一首，或者把文件拖进窗口</span>
        <div style="margin-top:12px"><button class="mp-btn" onclick={() => (lib.tracks.length ? player.playList(lib.tracks, 0) : ui.open('scan'))}>{lib.tracks.length ? `播放全部 ${lib.tracks.length} 首` : '扫描媒体库'}</button></div>
        <div style="margin-top:8px"><FxQuick /></div>
      {/if}
    </div>
  </div>
  <Lyrics />
  <div class="qpanel">
    <h6>接下来播放 · {player.order.length} 首</h6>
    <div class="qlist">
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
  </div>
</section>
