<script lang="ts">
  // 全屏背景动效。OLED 纯黑、迷你播放器、全屏覆盖层、窗口不可见时完全停下。
  import { onMount } from 'svelte';
  import { fx, fxLoop, fxFrameMs } from '../../lib/fx.svelte';
  import { player } from '../../lib/player.svelte';
  import { ui } from '../../lib/ui.svelte';
  import { BgDriver } from './driver';

  let box: HTMLDivElement;
  let lo: HTMLCanvasElement;
  let hi: HTMLCanvasElement;
  let driver = $state<BgDriver | null>(null);
  let hiOn = $state(false);

  const run = $derived(!ui.oled.black && fx.visible && !ui.mini && ui.videoId == null && ui.bookId == null && ui.comicId == null);

  onMount(() => {
    const d = new BgDriver(lo, hi, v => (hiOn = v));
    const size = () => {
      d.resize(box.clientWidth, box.clientHeight, fx.eco);
      d.frame(0); // 改尺寸会清空画布，立刻补一帧
    };
    const ro = new ResizeObserver(size);
    ro.observe(box);
    size();
    driver = d;
    return () => ro.disconnect();
  });

  // 省电档切换分辨率
  $effect(() => {
    fx.eco;
    if (driver) {
      driver.resize(box.clientWidth, box.clientHeight, fx.eco);
      driver.frame(0);
    }
  });

  // 封面模式：跟着当前曲目换图
  $effect(() => {
    if (driver && fx.s.bg === 'cover') driver.setCover(player.current?.cover ?? null);
  });

  // 逐帧循环：条件或设置变化时重启；静态模式画到稳定后自己停下
  $effect(() => {
    const d = driver;
    if (!d || !run) return;
    // 这些变化需要重新画（静态模式下循环可能已经停了）
    void [fx.s.bg, fx.s.intensity, fx.s.speed, fx.s.followCover, fx.s.hue, fx.s.fps, fx.reduced, player.current?.id];
    return fxLoop((_now, dt) => d.frame(dt), () => fxFrameMs(ui.dimmed));
  });
</script>

<div class="bg mp-bgfx" bind:this={box} data-fx={fx.s.bg}>
  <canvas class="fx-lo" bind:this={lo}></canvas>
  <canvas class="fx-hi" class:on={hiOn} bind:this={hi}></canvas>
</div>
