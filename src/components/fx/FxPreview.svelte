<script lang="ts">
  // 设置页里的小预览：用和正式背景相同的绘制，画在一个缩小的画布上
  import { onMount } from 'svelte';
  import { fx, fxLoop } from '../../lib/fx.svelte';
  import { player } from '../../lib/player.svelte';
  import Cover from '../Cover.svelte';
  import { BgDriver } from './driver';

  let box: HTMLDivElement;
  let lo: HTMLCanvasElement;
  let hi: HTMLCanvasElement;
  let driver = $state<BgDriver | null>(null);
  let hiOn = $state(false);
  const c = $derived(player.current);

  onMount(() => {
    const d = new BgDriver(lo, hi, v => (hiOn = v));
    const size = () => {
      // 预览很小，提高底层分辨率比例，免得看起来太糊
      d.resize(box.clientWidth * 2.5, box.clientHeight * 2.5, false, 1);
      d.frame(0);
    };
    const ro = new ResizeObserver(size);
    ro.observe(box);
    size();
    driver = d;
    return () => ro.disconnect();
  });

  $effect(() => {
    if (driver && fx.s.bg === 'cover') driver.setCover(player.current?.cover ?? null);
  });

  $effect(() => {
    const d = driver;
    if (!d || !fx.visible) return;
    void [fx.s.bg, fx.s.intensity, fx.s.speed, fx.s.followCover, fx.s.hue, fx.s.fps, fx.reduced, player.current?.id];
    return fxLoop((_now, dt) => d.frame(dt), () => fx.frameMs);
  });
</script>

<div class="fxp" bind:this={box}>
  <canvas class="lo" bind:this={lo}></canvas>
  <canvas class="hi" class:on={hiOn} bind:this={hi}></canvas>
  <div class="mini" class:round={fx.s.deck === 'vinyl'} class:ring={fx.s.ring}>
    <Cover tag="i" cls="" cover={c?.cover} name={c?.album ?? ''} hue={c?.hue} />
  </div>
</div>

<style>
  .fxp {
    position: relative;
    height: 150px;
    border-radius: 12px;
    overflow: hidden;
    background: #0b0b10;
    border: 1px solid var(--mp-line);
    contain: strict;
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
  }
  .hi {
    display: none;
  }
  .hi.on {
    display: block;
  }
  .mini {
    position: absolute;
    left: 50%;
    top: 50%;
    width: 58px;
    height: 58px;
    translate: -50% -50%;
    border-radius: 8px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
  }
  .mini.round {
    border-radius: 50%;
  }
  .mini.ring {
    outline: 2px dotted var(--mp-accent);
    outline-offset: 7px;
  }
  .mini :global(i) {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background-size: cover;
    background-position: center;
  }
</style>
