// 背景动效的绘制（canvas 2D）。正式背景和设置里的小预览共用。
// 两层画布：lo 是低分辨率底层（光斑、封面模糊，靠放大自然变柔和，省 GPU），
// hi 是正常分辨率的上层（粒子、波浪），只在需要时画。
import type { BgMode } from '../../lib/fx.svelte';

export interface Levels {
  bass: number;
  level: number;
  beat: number;
}

export interface FrameIn {
  mode: BgMode;
  /** 当前色相（已经平滑过渡） */
  hue: number;
  /** 动画时间（秒，已乘速度；静止时不前进） */
  t: number;
  dt: number;
  /** 强度倍率 */
  k: number;
  lv: Levels;
  bands: number[] | null;
  cover: HTMLImageElement | null;
  /** 封面淡入进度 0–1 */
  coverMix: number;
  prevCover: HTMLImageElement | null;
  eco: boolean;
}

const BASE = '#0b0b10';
const hsla = (h: number, s: number, l: number, a: number) => `hsl(${h.toFixed(1)} ${s}% ${l}% / ${a.toFixed(3)})`;

// 光斑：位置（0–1）、半径（相对长边）、色相偏移、饱和度、亮度、各自的漂移频率和相位
const BLOBS = [
  { x: 0.16, y: 0.18, r: 0.5, h: 0, s: 85, l: 52, f: [0.13, 0.09, 0.11], p: [0, 1.3, 2.1] },
  { x: 0.86, y: 0.84, r: 0.55, h: 70, s: 80, l: 47, f: [0.1, 0.12, 0.08], p: [2.2, 0.4, 4.0] },
  { x: 0.5, y: 0.55, r: 0.34, h: 300, s: 75, l: 45, f: [0.08, 0.15, 0.13], p: [4.1, 2.7, 0.9] },
  { x: 0.78, y: 0.12, r: 0.3, h: 35, s: 85, l: 50, f: [0.16, 0.07, 0.1], p: [1.1, 5.2, 3.3] },
];

/** 极光光斑；pulse 时随低频放大、随响度变亮 */
export function drawAurora(g: CanvasRenderingContext2D, w: number, h: number, f: FrameIn, dim = 1, pulse = false) {
  const { t, hue, k, lv } = f;
  g.globalCompositeOperation = 'source-over';
  g.globalAlpha = 1;
  g.fillStyle = BASE;
  g.fillRect(0, 0, w, h);
  g.globalCompositeOperation = 'lighter';
  const M = Math.max(w, h);
  const bass = pulse ? lv.bass : 0,
    level = pulse ? lv.level : 0.55;
  for (let i = 0; i < BLOBS.length; i++) {
    const b = BLOBS[i];
    const x = (b.x + 0.17 * Math.sin(t * b.f[0] + b.p[0]) + 0.07 * Math.sin(t * b.f[1] * 1.7 + b.p[1])) * w;
    const y = (b.y + 0.15 * Math.cos(t * b.f[1] + b.p[1]) + 0.06 * Math.sin(t * b.f[2] * 1.9 + b.p[2])) * h;
    let r = b.r * M * (1 + 0.15 * Math.sin(t * b.f[2] + b.p[2]));
    if (pulse) r *= 1 + (i % 2 ? 0.25 : 0.42) * bass * Math.min(1.4, k) + 0.12 * lv.beat * (i === 2 ? 1 : 0.3);
    const sx = 1 + 0.28 * Math.sin(t * b.f[0] * 0.8 + b.p[2]);
    const a = 0.36 * k * dim * (pulse ? 0.55 + 0.9 * level : 1);
    g.setTransform(sx, 0, 0, 1 / sx, x, y);
    g.rotate(t * b.f[1] * 0.4 + b.p[0]);
    const grd = g.createRadialGradient(0, 0, 0, 0, 0, r);
    const hh = hue + b.h;
    grd.addColorStop(0, hsla(hh, b.s, b.l, Math.min(1, a)));
    grd.addColorStop(0.45, hsla(hh, b.s, b.l, Math.min(1, a * 0.55)));
    grd.addColorStop(1, hsla(hh, b.s, b.l, 0));
    g.fillStyle = grd;
    g.fillRect(-r, -r, r * 2, r * 2);
  }
  g.setTransform(1, 0, 0, 1, 0, 0);
  g.globalCompositeOperation = 'source-over';
}

function drawImg(g: CanvasRenderingContext2D, img: HTMLImageElement, w: number, h: number, t: number, a: number) {
  const iw = img.naturalWidth || 1,
    ih = img.naturalHeight || 1;
  const sc = Math.max(w / iw, h / ih) * (1.28 + 0.1 * Math.sin(t * 0.11));
  const dw = iw * sc,
    dh = ih * sc;
  const ox = 0.05 * w * Math.sin(t * 0.07),
    oy = 0.05 * h * Math.cos(t * 0.09);
  g.globalAlpha = a;
  g.drawImage(img, (w - dw) / 2 + ox, (h - dh) / 2 + oy, dw, dh);
  g.globalAlpha = 1;
}

/** 封面模糊铺满 */
export function drawCoverBg(g: CanvasRenderingContext2D, w: number, h: number, f: FrameIn) {
  if (!f.cover && !f.prevCover) return drawAurora(g, w, h, f);
  g.globalCompositeOperation = 'source-over';
  g.fillStyle = BASE;
  g.fillRect(0, 0, w, h);
  g.filter = `blur(${Math.max(2, Math.round(w / 28))}px) saturate(1.3)`;
  if (f.prevCover && f.coverMix < 1) drawImg(g, f.prevCover, w, h, f.t, 1);
  if (f.cover) drawImg(g, f.cover, w, h, f.t, f.prevCover ? f.coverMix : 1);
  g.filter = 'none';
  // 压暗，保证文字清楚；强度越高压得越少
  g.fillStyle = `rgba(8,8,14,${Math.max(0.12, 0.62 - 0.3 * f.k).toFixed(3)})`;
  g.fillRect(0, 0, w, h);
}

// ---------------------------------------------------------------- 粒子星河

interface Star {
  r: number;
  a: number;
  s: number;
  tw: number;
  ph: number;
  hue: number;
  pop: number;
}

export class Stars {
  private list: Star[] = [];
  private sprites: HTMLCanvasElement[] = [];
  private spriteHue = -999;
  private n = 0;

  private ensure(n: number) {
    if (this.n === n) return;
    this.n = n;
    this.list = Array.from({ length: n }, () => {
      const r = Math.pow(Math.random(), 0.65);
      return {
        r,
        a: Math.random() * Math.PI * 2,
        s: 0.5 + Math.pow(Math.random(), 3) * 2.6,
        tw: 0.6 + Math.random() * 2.4,
        ph: Math.random() * 6.28,
        hue: (Math.random() * 80 - 20) | 0,
        pop: Math.random() < 0.35 ? 1 : 0,
      };
    });
  }

  private makeSprites(hue: number) {
    const h = Math.round(hue / 4) * 4;
    if (h === this.spriteHue) return;
    this.spriteHue = h;
    this.sprites = [-20, 20, 60].map(o => {
      const c = document.createElement('canvas');
      c.width = c.height = 32;
      const g = c.getContext('2d')!;
      const grd = g.createRadialGradient(16, 16, 0, 16, 16, 16);
      grd.addColorStop(0, 'rgba(255,255,255,1)');
      grd.addColorStop(0.18, hsla(h + o, 90, 85, 0.9));
      grd.addColorStop(0.45, hsla(h + o, 90, 65, 0.25));
      grd.addColorStop(1, hsla(h + o, 90, 60, 0));
      g.fillStyle = grd;
      g.fillRect(0, 0, 32, 32);
      return c;
    });
  }

  draw(g: CanvasRenderingContext2D, w: number, h: number, f: FrameIn, scale: number) {
    this.ensure(f.eco ? 140 : 300);
    this.makeSprites(f.hue);
    g.clearRect(0, 0, w, h);
    g.globalCompositeOperation = 'lighter';
    const cx = w * 0.5,
      cy = h * 0.52,
      R = Math.hypot(w, h) * 0.55;
    const spin = f.dt * (0.6 + f.lv.level * 0.8);
    for (const s of this.list) {
      // 内圈转得快，外圈慢，像星系
      s.a += spin * (0.012 + 0.03 / (s.r + 0.25));
      const x = cx + Math.cos(s.a) * s.r * R,
        y = cy + Math.sin(s.a) * s.r * R * 0.62;
      if (x < -20 || y < -20 || x > w + 20 || y > h + 20) continue;
      const tw = 0.35 + 0.65 * (0.5 + 0.5 * Math.sin(f.t * s.tw + s.ph));
      const a = Math.min(1, tw * (0.45 + 0.55 * f.k) + s.pop * f.lv.beat * 0.8);
      const sz = s.s * scale * (7 + s.pop * f.lv.beat * 5 + f.lv.bass * 2.5);
      g.globalAlpha = a;
      g.drawImage(this.sprites[s.hue < 0 ? 0 : s.hue < 30 ? 1 : 2], x - sz / 2, y - sz / 2, sz, sz);
    }
    g.globalAlpha = 1;
    g.globalCompositeOperation = 'source-over';
  }
}

// ---------------------------------------------------------------- 频谱波浪

export function drawWave(g: CanvasRenderingContext2D, w: number, h: number, f: FrameIn) {
  g.clearRect(0, 0, w, h);
  // 相邻频段做两次平滑，波形更柔和
  let bands = f.bands ?? [];
  for (let pass = 0; pass < 2; pass++) bands = bands.map((v, i) => (bands[Math.max(0, i - 1)] + v * 2 + bands[Math.min(bands.length - 1, i + 1)]) / 4);
  const n = bands.length || 1;
  // 频谱在横向镜像：中间低频，两边高频
  const at = (x: number) => {
    const p = Math.abs(x * 2 - 1) * (n - 1);
    const i = Math.floor(p),
      fr = p - i;
    const a = bands[i] ?? 0,
      b = bands[Math.min(n - 1, i + 1)] ?? 0;
    return a + (b - a) * (fr * fr * (3 - 2 * fr));
  };
  // 画布比窗口大 10%（像素位移留边），底部还有播放栏，波浪要从 1/5 高度起
  const layers = [
    { amp: 0.34, base: 0.24, off: 0, a: 0.18, dh: 70, sp: 0.35 },
    { amp: 0.26, base: 0.2, off: 2.1, a: 0.22, dh: 0, sp: -0.5 },
    { amp: 0.18, base: 0.16, off: 4.2, a: 0.28, dh: -40, sp: 0.8 },
  ];
  const step = Math.max(4, Math.round(w / (f.eco ? 48 : 96)));
  g.globalCompositeOperation = 'lighter';
  for (const L of layers) {
    g.beginPath();
    g.moveTo(0, h);
    for (let x = 0; x <= w + step; x += step) {
      const u = x / w;
      const v = at(Math.min(1, u));
      const y = h - h * (L.base + L.amp * v * Math.min(1.5, f.k) * 0.8 + 0.025 * Math.sin(u * 7 + f.t * L.sp * 3 + L.off));
      g.lineTo(x, y);
    }
    g.lineTo(w, h);
    g.closePath();
    const grd = g.createLinearGradient(0, h * 0.3, 0, h);
    grd.addColorStop(0, hsla(f.hue + L.dh, 90, 65, 0));
    grd.addColorStop(0.5, hsla(f.hue + L.dh, 90, 62, L.a * 0.6 * f.k));
    grd.addColorStop(1, hsla(f.hue + L.dh, 90, 55, L.a * f.k));
    g.fillStyle = grd;
    g.fill();
  }
  g.globalCompositeOperation = 'source-over';
}

/** 一帧：按模式画到两层画布上。返回 hi 层是否在用（不用时调用方可以隐藏它） */
export function renderFrame(lo: CanvasRenderingContext2D, hi: CanvasRenderingContext2D | null, f: FrameIn, stars: Stars, hiScale: number): boolean {
  const lw = lo.canvas.width,
    lh = lo.canvas.height;
  switch (f.mode) {
    case 'black':
      lo.fillStyle = '#000';
      lo.fillRect(0, 0, lw, lh);
      return false;
    case 'cover':
      drawCoverBg(lo, lw, lh, f);
      return false;
    case 'pulse':
      drawAurora(lo, lw, lh, f, 1, true);
      return false;
    case 'stars':
      drawAurora(lo, lw, lh, f, 0.45);
      if (hi) stars.draw(hi, hi.canvas.width, hi.canvas.height, f, hiScale);
      return true;
    case 'wave':
      drawAurora(lo, lw, lh, f, 0.6);
      if (hi) drawWave(hi, hi.canvas.width, hi.canvas.height, f);
      return true;
    default:
      drawAurora(lo, lw, lh, f);
      return false;
  }
}
