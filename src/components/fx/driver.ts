// 背景动效的状态驱动：色相平滑过渡、封面图片加载与淡入、动画时间累计。正式背景和设置预览共用。
import { coverUrl } from '../../lib/api';
import { fx, animatedBg, type BgMode } from '../../lib/fx.svelte';
import { player } from '../../lib/player.svelte';
import { renderFrame, Stars, type FrameIn } from './render';

/** 当前封面主色（播放器切歌时写在 :root 的 --hue 上） */
export function coverHue(): number {
  const v = parseFloat(document.documentElement.style.getPropertyValue('--hue'));
  return isFinite(v) ? v : 200;
}

const QUIET = { bass: 0, level: 0.5, beat: 0 };

export class BgDriver {
  private glo: CanvasRenderingContext2D;
  private ghi: CanvasRenderingContext2D | null;
  private stars = new Stars();
  private t = 40 + Math.random() * 200;
  private hue = NaN;
  private cover: HTMLImageElement | null = null;
  private prev: HTMLImageElement | null = null;
  private mix = 1;
  private coverName: string | null | undefined = undefined;
  private loading = false;
  hiOn = false;
  hiScale = 1;

  constructor(
    public lo: HTMLCanvasElement,
    public hi: HTMLCanvasElement | null,
    private onHi?: (on: boolean) => void,
  ) {
    this.glo = lo.getContext('2d', { alpha: false })!;
    this.ghi = hi?.getContext('2d') ?? null;
  }

  /** 按显示尺寸（CSS 像素）设置画布分辨率。lo 层很低（放大后自然柔和），hi 层接近原生 */
  resize(cssW: number, cssH: number, eco: boolean, maxHiDpr = 1.25) {
    const ls = eco ? 1 / 10 : 1 / 6;
    const lw = Math.max(8, Math.round(cssW * ls)),
      lh = Math.max(8, Math.round(cssH * ls));
    if (this.lo.width !== lw || this.lo.height !== lh) {
      this.lo.width = lw;
      this.lo.height = lh;
    }
    if (this.hi) {
      const s = Math.min(devicePixelRatio || 1, maxHiDpr) * (eco ? 0.6 : 1);
      const hw = Math.max(8, Math.round(cssW * s)),
        hh = Math.max(8, Math.round(cssH * s));
      this.hiScale = s;
      if (this.hi.width !== hw || this.hi.height !== hh) {
        this.hi.width = hw;
        this.hi.height = hh;
      }
    }
  }

  /** 封面模式用：换封面时加载新图，加载完淡入 */
  setCover(name: string | null) {
    if (name === this.coverName) return;
    this.coverName = name;
    if (!name) {
      this.prev = this.cover;
      this.cover = null;
      this.mix = 0;
      return;
    }
    const img = new Image();
    img.decoding = 'async';
    this.loading = true;
    img.onload = () => {
      if (this.coverName !== name) return;
      this.loading = false;
      this.prev = this.cover;
      this.cover = img;
      this.mix = this.prev ? 0 : 1;
    };
    img.onerror = () => {
      if (this.coverName !== name) return;
      this.loading = false;
      this.prev = null;
      this.cover = null;
    };
    img.src = coverUrl(name);
  }

  /** 画一帧。返回 false 表示画面已经稳定、可以停下（静态模式） */
  frame(dt: number, mode: BgMode = fx.s.bg): boolean {
    const s = fx.s;
    const animated = animatedBg(mode) && !fx.reduced;
    const target = s.followCover ? coverHue() : s.hue;
    if (!isFinite(this.hue)) this.hue = target;
    // 色相走最短路径，约 2 秒过渡完
    let d = ((target - this.hue + 540) % 360) - 180;
    this.hue = Math.abs(d) < 0.3 ? target : this.hue + d * Math.min(1, dt * 1.8);
    d = ((target - this.hue + 540) % 360) - 180;
    if (this.mix < 1) this.mix = Math.min(1, this.mix + dt / 1.4);
    if (this.mix >= 1) this.prev = null;
    if (animated) this.t += dt * fx.speedMul;
    const live = mode === 'pulse' || mode === 'stars' || mode === 'wave';
    const f: FrameIn = {
      mode,
      hue: this.hue,
      t: this.t,
      dt: animated ? dt * fx.speedMul : 0,
      k: fx.intensityMul,
      lv: live && animated ? player.levels() : QUIET,
      bands: mode === 'wave' ? (animated ? player.fxBands(24) : new Array(24).fill(0.15)) : null,
      cover: this.cover,
      prevCover: this.prev,
      coverMix: this.mix,
      eco: fx.eco,
    };
    const hiOn = renderFrame(this.glo, this.ghi, f, this.stars, this.hiScale);
    if (hiOn !== this.hiOn) {
      this.hiOn = hiOn;
      if (!hiOn && this.ghi) this.ghi.clearRect(0, 0, this.hi!.width, this.hi!.height);
      this.onHi?.(hiOn);
    }
    return animated || Math.abs(d) > 0.3 || this.mix < 1 || this.loading;
  }
}
