<script lang="ts">
  // 歌词：按设置里的“歌词动效”分发到各样式（components/lyrics/*）。
  // 计时只在播放且窗口可见时逐帧更新；暂停、拖进度时跟着 player.t 更新一次。
  // 系统开启“减少动态效果”时降级为无动画的逐行高亮。
  import { player } from '../lib/player.svelte';
  import { lyricsfx, lineAt, DEMO_LINES, DEMO_LEN } from '../lib/lyricsfx.svelte';
  import { clamp } from '../lib/util';
  import { fx } from '../lib/fx.svelte';
  import LineList from './lyrics/LineList.svelte';
  import Focus from './lyrics/Focus.svelte';
  import StylePicker from './lyrics/StylePicker.svelte';

  let { cls = 'lyr', pos = 0.42, demo = false, picker = true }: { cls?: string; pos?: number; demo?: boolean; picker?: boolean } = $props();

  lyricsfx.load();
  const cfg = $derived(lyricsfx.cfg);
  const lines = $derived(demo ? DEMO_LINES : player.lyrics);
  const timed = $derived(lines.length > 0 && lines[0].t >= 0);
  const dur = $derived(demo ? DEMO_LEN : player.dur);

  const mq = typeof matchMedia === 'function' ? matchMedia('(prefers-reduced-motion: reduce)') : null;
  let reduced = $state(mq?.matches ?? false);
  let visible = $state(typeof document === 'undefined' || document.visibilityState !== 'hidden');
  $effect(() => {
    const onMq = () => (reduced = mq?.matches ?? false);
    const onVis = () => (visible = document.visibilityState !== 'hidden');
    mq?.addEventListener('change', onMq);
    document.addEventListener('visibilitychange', onVis);
    return () => {
      mq?.removeEventListener('change', onMq);
      document.removeEventListener('visibilitychange', onVis);
    };
  });

  const mode = $derived(reduced || !timed ? 'plain' : cfg.style);
  /** 逐帧更新：播放中（设置页预览一直播放）、窗口可见、没有要求减少动态效果 */
  const running = $derived(visible && !reduced && timed && (demo || player.playing));

  let now = $state(0);
  let beat = $state(0);
  const k = $derived(timed ? lineAt(lines, now) : -1);

  // 不逐帧更新时跟着 player.t（暂停中拖进度、换歌、减少动态效果）；窗口不可见时不更新
  $effect(() => {
    if (running || demo || !visible) return;
    now = player.t;
  });

  // mpv 引擎的时间是事件推送的，帧间按播放速度外推，让逐字填色顺滑
  let baseT = -1,
    basePerf = 0,
    slow = 0,
    bbuf: Uint8Array<ArrayBuffer> | null = null;

  function clock(perf: number): number {
    if (demo) return ((perf / 1000) % DEMO_LEN) - 0.4;
    const t = player.t;
    if (t !== baseT) {
      baseT = t;
      basePerf = perf;
    }
    const est = t + Math.min(0.5, ((perf - basePerf) / 1000) * player.speed);
    // 外推得比下一次事件快一点时不往回跳
    return est < now && now - est < 0.3 ? now : est;
  }

  /** 低频起音（0–1）：读 Web Audio 分析器 40–160 Hz，不改 player 的频谱平滑状态；没有分析器时用 120 BPM 的轻脉冲代替 */
  function bass(t: number): number {
    const a = player.analyser,
      ctx = player.ctx;
    if (demo || !a || !ctx || ctx.state !== 'running' || player.backend !== 'web') {
      return Math.pow(Math.max(0, Math.cos(t * Math.PI * 2 * 2)), 12) * 0.8;
    }
    if (!bbuf || bbuf.length !== a.frequencyBinCount) bbuf = new Uint8Array(new ArrayBuffer(a.frequencyBinCount));
    a.getByteFrequencyData(bbuf);
    const hz = ctx.sampleRate / a.fftSize,
      lo = Math.max(1, Math.round(40 / hz)),
      hi = Math.max(lo + 1, Math.round(160 / hz));
    let m = 0;
    for (let i = lo; i <= hi; i++) m += bbuf[i];
    const lvl = m / (hi - lo + 1) / 255;
    const onset = Math.max(0, lvl - slow * 1.04);
    slow = slow * 0.94 + lvl * 0.06;
    return clamp(onset * 5);
  }

  $effect(() => {
    if (!running) return;
    const beating = cfg.style === 'bounce';
    let raf = 0,
      last = 0;
    const tick = (perf: number) => {
      // 高刷新率屏幕上按动效设置限帧（默认 60，省电 20）
      if (last && perf - last < Math.max(fx.frameMs, 16.6) - 3) {
        raf = requestAnimationFrame(tick);
        return;
      }
      last = perf;
      now = clock(perf);
      if (beating) {
        const b = bass(now);
        beat = b > beat ? b : beat * 0.86;
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(raf);
      beat = 0;
    };
  });

  function seek(t: number) {
    if (demo) return;
    player.seek(t);
    if (!player.playing) player.play();
  }
</script>

<div
  class="{cls} ly-wrap ly-{mode}"
  class:noblur={!cfg.blur}
  class:reduced
  data-drift
  style:--ly-size={cfg.size}
  style:--ly-align={cfg.align === 'left' ? 'start' : cfg.align === 'right' ? 'end' : 'center'}
  style:--ly-origin={cfg.align}
  style:--ly-vorigin={cfg.align === 'left' ? 'top' : cfg.align === 'right' ? 'bottom' : 'center'}
>
  {#if !lines.length}
    <div class="ly-box" data-mp="lyrics">
      <div class="mp-ly-inner">
        <div class="mp-ly-empty">{player.current ? '纯音乐 / 暂无歌词' : '还没有在播放'}<br /><small style="font-size:.5em;font-weight:400">把同名 .lrc 文件放在歌曲旁边即可显示歌词</small></div>
      </div>
    </div>
  {:else if mode === 'fade' || mode === 'typewriter'}
    <Focus {lines} {k} {now} {dur} {mode} {pos} onseek={seek} />
  {:else}
    <LineList {lines} {k} {now} {dur} {mode} {timed} {pos} {beat} onseek={seek} />
  {/if}
  {#if picker && !demo}<StylePicker />{/if}
</div>
