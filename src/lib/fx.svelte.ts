// 背景动效与封面区动效的设置（保存在设置键 'fx'），以及动效共用的“该不该动”判断。
import { invoke, inTauri } from './api';

export type BgMode = 'aurora' | 'pulse' | 'stars' | 'wave' | 'cover' | 'static' | 'black';
export type DeckMode = 'vinyl' | 'breathe' | 'still';
export type FpsMode = 60 | 30 | 'eco';

export interface FxSettings {
  bg: BgMode;
  /** 强度 0–100 */
  intensity: number;
  /** 速度 0–100（50 为默认速度） */
  speed: number;
  /** 跟随封面取色；关闭时用 hue */
  followCover: boolean;
  hue: number;
  fps: FpsMode;
  deck: DeckMode;
  /** 环形频谱 */
  ring: boolean;
}

export const FX_DEFAULT: FxSettings = { bg: 'aurora', intensity: 60, speed: 50, followCover: true, hue: 220, fps: 60, deck: 'vinyl', ring: true };

export const BG_MODES: { id: BgMode; name: string; desc: string }[] = [
  { id: 'aurora', name: '流动极光', desc: '几团光斑缓慢漂移变形，颜色取自封面' },
  { id: 'pulse', name: '随音乐律动', desc: '光斑大小和亮度跟着低频与响度变化' },
  { id: 'stars', name: '粒子星河', desc: '粒子缓慢旋转，随节拍闪烁' },
  { id: 'wave', name: '频谱波浪', desc: '底部柔和的频谱波形' },
  { id: 'cover', name: '封面模糊', desc: '当前封面模糊铺满，缓慢缩放移动' },
  { id: 'static', name: '静态', desc: '静止的渐变光斑，不占用资源' },
  { id: 'black', name: '纯黑', desc: '纯黑背景，适合 OLED 屏幕' },
];

export const DECK_MODES: { id: DeckMode; name: string; desc: string }[] = [
  { id: 'vinyl', name: '旋转黑胶', desc: '播放时唱片滑出并旋转，暂停时缓慢停转' },
  { id: 'breathe', name: '封面呼吸', desc: '方形封面随音乐轻微呼吸' },
  { id: 'still', name: '静止封面', desc: '只显示方形封面' },
];

export const FPS_MODES: { id: FpsMode; name: string }[] = [
  { id: 60, name: '60 帧' },
  { id: 30, name: '30 帧' },
  { id: 'eco', name: '省电' },
];

/** 动效模式是否需要逐帧刷新 */
export const animatedBg = (m: BgMode) => m !== 'static' && m !== 'black';

const clampN = (v: unknown, lo: number, hi: number, d: number) => (typeof v === 'number' && isFinite(v) ? Math.min(hi, Math.max(lo, v)) : d);

/** 合并默认值并修正非法值（旧版本或手改的设置） */
export function normalizeFx(raw: unknown): FxSettings {
  const r = (raw && typeof raw === 'object' ? raw : {}) as Partial<FxSettings>;
  const s = { ...FX_DEFAULT };
  if (BG_MODES.some(m => m.id === r.bg)) s.bg = r.bg!;
  if (DECK_MODES.some(m => m.id === r.deck)) s.deck = r.deck!;
  if (FPS_MODES.some(m => m.id === r.fps)) s.fps = r.fps!;
  s.intensity = clampN(r.intensity, 0, 100, s.intensity);
  s.speed = clampN(r.speed, 0, 100, s.speed);
  s.hue = clampN(r.hue, 0, 360, s.hue);
  if (typeof r.followCover === 'boolean') s.followCover = r.followCover;
  if (typeof r.ring === 'boolean') s.ring = r.ring;
  return s;
}

class Fx {
  s = $state<FxSettings>({ ...FX_DEFAULT });
  /** 窗口是否可见（没有最小化、没有隐藏到托盘、页面没有切到后台） */
  visible = $state(true);
  /** 系统开启了“减少动态效果” */
  reduced = $state(false);
  private timer: ReturnType<typeof setTimeout> | undefined;

  /** 速度滑块换算成时间倍率：0 → 0.25 倍，50 → 1 倍，100 → 4 倍 */
  get speedMul() {
    return Math.pow(2, (this.s.speed - 50) / 25);
  }
  /** 强度滑块换算成倍率：0 → 0.35，60 → 1，100 → 1.43 */
  get intensityMul() {
    return 0.35 + this.s.intensity * 0.0108;
  }
  /** 帧间隔（毫秒）。高刷新率屏幕上 requestAnimationFrame 会跑到 120/144/175 次，必须显式限帧 */
  get frameMs() {
    return this.s.fps === 60 ? 1000 / 60 : this.s.fps === 30 ? 1000 / 30 : 1000 / 20;
  }
  get eco() {
    return this.s.fps === 'eco';
  }

  async load() {
    if (!inTauri) return;
    try {
      const v = await invoke<unknown>('get_setting', { key: 'fx' });
      if (v != null) this.s = normalizeFx(v);
    } catch {}
  }

  /** 改设置：立即生效，稍后保存（拖滑块时不会连续写库） */
  set(patch: Partial<FxSettings>) {
    this.s = normalizeFx({ ...this.s, ...patch });
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      if (inTauri) invoke('set_setting', { key: 'fx', value: $state.snapshot(this.s) }).catch(() => {});
    }, 300);
  }
}

export const fx = new Fx();

/**
 * 逐帧循环（带帧率上限）。返回停止函数。draw 返回 false 时自动停下（例如静态背景画完一帧）。
 * 是否运行由调用方在 $effect 里决定：条件变化时 effect 重跑，先停旧循环再开新循环。
 */
export function fxLoop(draw: (now: number, dt: number) => boolean | void, frameMs: () => number): () => void {
  let raf = 0,
    timer: ReturnType<typeof setTimeout> | undefined,
    last = 0,
    stopped = false;
  const tick = (now: number) => {
    raf = 0;
    if (stopped) return;
    const ms = frameMs();
    // 留一点余量：60 Hz 屏幕上 16.6 ms 的间隔抖动不会被误跳过
    if (last && ms && now - last < ms - 3) {
      raf = requestAnimationFrame(tick);
      return;
    }
    const dt = last ? Math.min(0.1, (now - last) / 1000) : 1 / 60;
    last = now;
    if (draw(now, dt) === false) return;
    // 高刷新率屏幕上不每次 vsync 都醒：先睡到下一帧附近，再对齐到 vsync
    const wait = ms - (performance.now() - now) - 6;
    if (wait > 1) timer = setTimeout(() => (raf = requestAnimationFrame(tick)), wait);
    else raf = requestAnimationFrame(tick);
  };
  raf = requestAnimationFrame(tick);
  return () => {
    stopped = true;
    clearTimeout(timer);
    cancelAnimationFrame(raf);
  };
}

/** 屏幕自动变暗时降到 20 帧 */
export const fxFrameMs = (dimmed: boolean) => (dimmed ? Math.max(fx.frameMs, 50) : fx.frameMs);

if (typeof window !== 'undefined') {
  // 开发模式下方便自动化测试切换动效
  if (import.meta.env.DEV) (window as unknown as Record<string, unknown>).__fx = fx;
  fx.load();
  // 减少动态效果
  const mq = window.matchMedia?.('(prefers-reduced-motion: reduce)');
  if (mq) {
    fx.reduced = mq.matches;
    mq.addEventListener?.('change', () => (fx.reduced = mq.matches));
  }
  // 可见性：页面隐藏、窗口最小化或隐藏到托盘时暂停所有动效
  let docVisible = document.visibilityState !== 'hidden',
    winVisible = true;
  const apply = () => (fx.visible = docVisible && winVisible);
  document.addEventListener('visibilitychange', () => {
    docVisible = document.visibilityState !== 'hidden';
    apply();
  });
  if (inTauri) {
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      const w = getCurrentWindow();
      let busy = false;
      const check = async () => {
        if (busy) return;
        busy = true;
        try {
          const [min, vis] = await Promise.all([w.isMinimized(), w.isVisible()]);
          winVisible = vis && !min;
          apply();
        } catch {
        } finally {
          busy = false;
        }
      };
      w.onResized(check).catch(() => {});
      w.onFocusChanged(check).catch(() => {});
      // 隐藏到托盘不会触发 resize，低频轮询兜底（一次 IPC，开销可以忽略）
      setInterval(check, 2000);
      check();
    });
  }
}
