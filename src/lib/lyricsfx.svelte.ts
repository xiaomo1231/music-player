// 歌词动效：样式设置（保存在设置键 lyricsfx）和各样式共用的计时工具。
// 歌词组件见 components/Lyrics.svelte 和 components/lyrics/*。
import { invoke, inTauri, type LyricLine } from './api';
import { clamp } from './util';

export type LyricStyle = 'karaoke' | 'scroll' | 'fade' | 'typewriter' | 'bounce' | 'vertical';
export type LyricAlign = 'left' | 'center' | 'right';

export interface LyricsFxCfg {
  style: LyricStyle;
  /** 字号倍数，0.7–1.6 */
  size: number;
  align: LyricAlign;
  /** 是否模糊非当前行 */
  blur: boolean;
}

export const LYRIC_STYLES: { id: LyricStyle; name: string; desc: string }[] = [
  { id: 'scroll', name: '逐行放大滚动', desc: '当前行放大，上下行渐隐模糊' },
  { id: 'karaoke', name: '逐字高亮', desc: '卡拉 OK 式逐字填色' },
  { id: 'fade', name: '淡入上浮', desc: '一行一行从下方浮现' },
  { id: 'typewriter', name: '打字机', desc: '跟着演唱逐字打出' },
  { id: 'bounce', name: '弹性节拍', desc: '当前行随低音节拍弹动' },
  { id: 'vertical', name: '竖排', desc: '从右往左竖向排列' },
];

export const LYRIC_ALIGNS: { id: LyricAlign; name: string }[] = [
  { id: 'left', name: '左' },
  { id: 'center', name: '中' },
  { id: 'right', name: '右' },
];

const DEFAULTS: LyricsFxCfg = { style: 'scroll', size: 1, align: 'left', blur: true };
const KEY = 'lyricsfx';

/** 把保存的值和默认值合并，丢掉不认识的取值 */
export function mergeCfg(v: unknown): LyricsFxCfg {
  const o = (v && typeof v === 'object' ? v : {}) as Partial<Record<keyof LyricsFxCfg, unknown>>;
  const style = LYRIC_STYLES.some(s => s.id === o.style) ? (o.style as LyricStyle) : DEFAULTS.style;
  const align = LYRIC_ALIGNS.some(a => a.id === o.align) ? (o.align as LyricAlign) : DEFAULTS.align;
  const size = typeof o.size === 'number' && isFinite(o.size) ? clamp(o.size, 0.7, 1.6) : DEFAULTS.size;
  const blur = typeof o.blur === 'boolean' ? o.blur : DEFAULTS.blur;
  return { style, size, align, blur };
}

class LyricsFx {
  cfg = $state<LyricsFxCfg>({ ...DEFAULTS });
  private loaded = false;
  private ver = 0;

  async load() {
    if (this.loaded || !inTauri) return;
    this.loaded = true;
    const v = this.ver;
    try {
      const saved = await invoke('get_setting', { key: KEY });
      // 读取期间用户已经改过就不再覆盖
      if (v === this.ver) this.cfg = mergeCfg(saved);
    } catch {
      /* 读取失败时沿用默认值 */
    }
  }

  private timer: ReturnType<typeof setTimeout> | undefined;

  /** 改设置：立即生效，稍后保存（拖动字号滑块时不会每一步都写一次） */
  set(patch: Partial<LyricsFxCfg>) {
    this.cfg = mergeCfg({ ...this.cfg, ...patch });
    this.ver++;
    if (!inTauri) return;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      invoke('set_setting', { key: KEY, value: $state.snapshot(this.cfg) }).catch(() => {});
    }, 250);
  }
}

export const lyricsfx = new LyricsFx();

// ---------------------------------------------------------------- 计时

/** 当前歌词行（和 player.lyricIndex 相同：提前 0.15 秒切行）；没有时间轴时返回 -1 */
export function lineAt(lines: LyricLine[], t: number): number {
  if (!lines.length || lines[0].t < 0) return -1;
  let lo = 0,
    hi = lines.length - 1,
    k = -1;
  while (lo <= hi) {
    const m = (lo + hi) >> 1;
    if (lines[m].t <= t + 0.15) {
      k = m;
      lo = m + 1;
    } else hi = m - 1;
  }
  return k;
}

/** 一行里的一个演唱单位（一个汉字、一个英文单词或增强 LRC 里的一段） */
export interface Unit {
  text: string;
  t0: number;
  t1: number;
}

/** 把一行拆成汉字 / 英文单词（单词带上后面的空格） */
export function splitText(s: string): string[] {
  return s.match(/[A-Za-z0-9'’\-.,!?]+\s*|\s+|./gu) ?? [];
}

/** 演唱单位的“字数”（英文单词按字母数折算，避免长单词占时太短） */
const weight = (s: string) => {
  const w = s.trim();
  if (!w) return 0.3;
  return /^[A-Za-z0-9]/.test(w) ? Math.max(1, w.length * 0.45) : w.length;
};

/**
 * 一行的逐字时间。增强 LRC 直接用逐字时间；普通 LRC 按字数把这一行到下一行的时间线性分配
 * （间隔很长时按每字约 0.45 秒估算，避免间奏里还在慢慢填色）。
 */
export function lineUnits(lines: LyricLine[], k: number, dur: number): Unit[] {
  const l = lines[k];
  if (!l) return [];
  const next = lines[k + 1]?.t ?? (dur > l.t ? dur : l.t + 5);
  if (l.words.length) {
    return l.words.map(([t0, text], i) => ({ text, t0, t1: Math.max(t0 + 0.05, l.words[i + 1]?.[0] ?? next) }));
  }
  const parts = splitText(l.text);
  const total = parts.reduce((s, p) => s + weight(p), 0) || 1;
  const span = Math.max(0.3, Math.min(next - l.t, 1 + total * 0.45));
  let acc = 0;
  return parts.map(text => {
    const t0 = l.t + (acc / total) * span;
    acc += weight(text);
    return { text, t0, t1: l.t + (acc / total) * span };
  });
}

export const unitProgress = (u: Unit, t: number) => clamp((t - u.t0) / Math.max(0.05, u.t1 - u.t0));

/** 整行进度（0–1），按单位加权 */
export function lineProgress(units: Unit[], t: number): number {
  if (!units.length) return 0;
  let done = 0,
    total = 0;
  for (const u of units) {
    const w = weight(u.text);
    total += w;
    done += w * unitProgress(u, t);
  }
  return total ? done / total : 0;
}

// ---------------------------------------------------------------- 设置页预览用的示例歌词

export const DEMO_LINES: LyricLine[] = [
  { t: 0, text: '风从街角慢慢地吹过来', words: [] },
  { t: 3.2, text: '带走一点点犹豫', words: [] },
  { t: 6, text: 'Let the morning wind carry it', words: [] },
  { t: 9.4, text: '我们沿着河岸一直走', words: [] },
  { t: 12.6, text: '直到影子变得很短很短', words: [] },
];
export const DEMO_LEN = 16;
