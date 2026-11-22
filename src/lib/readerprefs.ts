// 漫画 / 小说阅读器的翻页方式等偏好：全局默认存在 settings 表（reader_comic / reader_novel），
// 每本书单独记住翻页方式和阅读方向（reader_comic:<id> / reader_novel:<id>）。纯逻辑，不含界面。
import { inTauri, invoke } from './api';

/** 漫画翻页方式：左右翻页 / 上下翻页 / 上下连续滚动（条漫）/ 左右连续滚动 */
export type ComicFlow = 'h' | 'v' | 'vscroll' | 'hscroll';
/** 阅读方向：从左到右 / 从右到左（日漫） */
export type ComicDir = 'ltr' | 'rtl';
/** 翻页动画：滑动 / 仿真翻书 / 无 */
export type PageAnim = 'slide' | 'flip' | 'none';
/** 页面适配：适应窗口 / 适应宽度 / 适应高度 / 原始大小 */
export type ComicFit = 'screen' | 'width' | 'height' | 'original';

export interface ComicPrefs {
  flow: ComicFlow;
  dir: ComicDir;
  /** 版式：单页 / 双页（只在翻页模式下用） */
  spread: 'single' | 'double';
  /** 双页时第一页（封面）单独显示 */
  coverAlone: boolean;
  anim: PageAnim;
  fit: ComicFit;
  /** 连续滚动：页间距（px） */
  gap: number;
  /** 连续滚动：页面宽度（上下滚动）/ 高度（左右滚动），占窗口的百分比 */
  size: number;
}

/** 每本书单独记住的部分 */
export type ComicBookPrefs = Pick<ComicPrefs, 'flow' | 'dir' | 'spread'>;

export const COMIC_DEFAULTS: ComicPrefs = {
  flow: 'h',
  dir: 'rtl',
  spread: 'single',
  coverAlone: true,
  anim: 'slide',
  fit: 'screen',
  gap: 8,
  size: 60,
};

export const COMIC_FLOWS: { id: ComicFlow; name: string; desc: string }[] = [
  { id: 'h', name: '左右翻页', desc: '一次一页，左右滑动翻页' },
  { id: 'v', name: '上下翻页', desc: '一次一页，上下滑动翻页' },
  { id: 'vscroll', name: '上下连续滚动', desc: '条漫 / 长条，页面首尾相接' },
  { id: 'hscroll', name: '左右连续滚动', desc: '页面左右相接，横向滚动' },
];

export const COMIC_FITS: { id: ComicFit; name: string }[] = [
  { id: 'screen', name: '适应窗口' },
  { id: 'width', name: '适应宽度' },
  { id: 'height', name: '适应高度' },
  { id: 'original', name: '原始大小' },
];

export const PAGE_ANIMS: { id: PageAnim; name: string }[] = [
  { id: 'slide', name: '滑动' },
  { id: 'flip', name: '仿真翻书' },
  { id: 'none', name: '无动画' },
];

/** 小说翻页方式：上下滚动 / 左右翻页 / 自动滚动 */
export type NovelMode = 'scroll' | 'page' | 'auto';

export interface NovelPrefs {
  mode: NovelMode;
  anim: PageAnim;
  /** 自动滚动速度（像素 / 秒） */
  speed: number;
}

export type NovelBookPrefs = Pick<NovelPrefs, 'mode'>;

export const NOVEL_DEFAULTS: NovelPrefs = { mode: 'scroll', anim: 'slide', speed: 40 };

export const NOVEL_MODES: { id: NovelMode; name: string; desc: string }[] = [
  { id: 'scroll', name: '上下滚动', desc: '整章连续，滚轮上下滚动' },
  { id: 'page', name: '左右翻页', desc: '按屏分页，点击左右或方向键翻页' },
  { id: 'auto', name: '自动滚动', desc: '匀速向下滚动，空格暂停' },
];

export const SPEED_MIN = 10;
export const SPEED_MAX = 300;

const pick = <T extends string>(v: unknown, ok: readonly T[], d: T): T => (ok.includes(v as T) ? (v as T) : d);
const num = (v: unknown, lo: number, hi: number, d: number) => (typeof v === 'number' && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : d);

const FLOW_IDS: ComicFlow[] = ['h', 'v', 'vscroll', 'hscroll'];
const FIT_IDS: ComicFit[] = ['screen', 'width', 'height', 'original'];
const ANIM_IDS: PageAnim[] = ['slide', 'flip', 'none'];
const NMODE_IDS: NovelMode[] = ['scroll', 'page', 'auto'];

/** 把存储里读出的任意值整理成合法的漫画偏好（字段缺失或不合法时用 d 里的） */
export function normComic(v: unknown, d: ComicPrefs = COMIC_DEFAULTS): ComicPrefs {
  const o = (v && typeof v === 'object' ? v : {}) as Record<string, unknown>;
  return {
    flow: pick(o.flow, FLOW_IDS, d.flow),
    dir: pick(o.dir, ['ltr', 'rtl'] as const, d.dir),
    spread: pick(o.spread, ['single', 'double'] as const, d.spread),
    coverAlone: typeof o.coverAlone === 'boolean' ? o.coverAlone : d.coverAlone,
    anim: pick(o.anim, ANIM_IDS, d.anim),
    fit: pick(o.fit, FIT_IDS, d.fit),
    gap: Math.round(num(o.gap, 0, 80, d.gap)),
    size: Math.round(num(o.size, 20, 100, d.size)),
  };
}

export function normNovel(v: unknown, d: NovelPrefs = NOVEL_DEFAULTS): NovelPrefs {
  const o = (v && typeof v === 'object' ? v : {}) as Record<string, unknown>;
  return {
    mode: pick(o.mode, NMODE_IDS, d.mode),
    anim: pick(o.anim, ANIM_IDS, d.anim),
    speed: Math.round(num(o.speed, SPEED_MIN, SPEED_MAX, d.speed)),
  };
}

async function getKey(key: string): Promise<unknown> {
  if (!inTauri) return null;
  try {
    return await invoke<unknown>('get_setting', { key });
  } catch {
    return null;
  }
}

async function setKey(key: string, value: unknown) {
  if (!inTauri) return;
  await invoke('set_setting', { key, value }).catch(() => {});
}

export const comicGlobal = async () => normComic(await getKey('reader_comic'));
export const saveComicGlobal = (p: ComicPrefs) => setKey('reader_comic', p);
export const novelGlobal = async () => normNovel(await getKey('reader_novel'));
export const saveNovelGlobal = (p: NovelPrefs) => setKey('reader_novel', p);

/** 这本书的偏好：全局默认 + 这本书自己记住的翻页方式和方向。has 表示这本书是否单独设置过 */
export async function comicFor(id: number): Promise<{ prefs: ComicPrefs; has: boolean }> {
  const [g, b] = await Promise.all([comicGlobal(), getKey(`reader_comic:${id}`)]);
  if (!b || typeof b !== 'object') return { prefs: g, has: false };
  const m = normComic(b, g);
  return { prefs: { ...g, flow: m.flow, dir: m.dir, spread: m.spread }, has: true };
}

export const saveComicBook = (id: number, p: ComicBookPrefs) => setKey(`reader_comic:${id}`, { flow: p.flow, dir: p.dir, spread: p.spread });

export async function novelFor(id: number): Promise<NovelPrefs> {
  const [g, b] = await Promise.all([novelGlobal(), getKey(`reader_novel:${id}`)]);
  if (!b || typeof b !== 'object') return g;
  return { ...g, mode: normNovel(b, g).mode };
}

export const saveNovelBook = (id: number, p: NovelBookPrefs) => setKey(`reader_novel:${id}`, { mode: p.mode });

// ---------- 漫画排版（纯函数） ----------

/**
 * 把页面分成“一屏”的组合。单页时每页一组；双页时两页一组，
 * 横向的跨页大图（宽 > 高）单独一组，coverAlone 时第一页单独一组。
 * ratios[i] 是第 i 页的 高 / 宽，未知时为 0（按竖版处理）。
 */
export function makeSpreads(count: number, ratios: ArrayLike<number>, double: boolean, coverAlone: boolean): number[][] {
  const out: number[][] = [];
  const wide = (i: number) => ratios[i] > 0 && ratios[i] < 1;
  let i = 0;
  while (i < count) {
    if (!double || wide(i) || (coverAlone && i === 0) || i + 1 >= count || wide(i + 1)) {
      out.push([i]);
      i++;
    } else {
      out.push([i, i + 1]);
      i += 2;
    }
  }
  return out;
}

/** 第 page 页在第几组 */
export function spreadOf(spreads: number[][], page: number): number {
  // 组是按页码递增的，二分查找
  let lo = 0;
  let hi = spreads.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (spreads[mid][0] <= page) lo = mid;
    else hi = mid - 1;
  }
  return Math.max(0, lo);
}

/**
 * 一组页面在舞台里的显示高度（各页同高，并排）。
 * ratios 是这一组每页的 高 / 宽，nat 是原图高度（原始大小用）。
 */
export function spreadHeight(fit: ComicFit, W: number, H: number, ratios: number[], natH: number): number {
  const sumW = ratios.reduce((s, r) => s + 1 / r, 0); // 高为 1 时的总宽度
  switch (fit) {
    case 'width':
      return W / sumW;
    case 'height':
      return H;
    case 'original':
      return natH > 0 ? natH : H;
    default:
      return Math.min(H, W / sumW);
  }
}

/** 连续滚动：前缀和。sizes[i] 是第 i 页沿滚动方向的长度，返回每页的起点和总长 */
export function offsets(sizes: number[], gap: number): { start: number[]; total: number } {
  const start = new Array<number>(sizes.length);
  let p = 0;
  for (let i = 0; i < sizes.length; i++) {
    start[i] = p;
    p += sizes[i] + gap;
  }
  return { start, total: Math.max(0, p - gap) };
}

/** 位置 pos 落在哪一页（start 递增） */
export function pageAt(start: number[], pos: number): number {
  let lo = 0;
  let hi = start.length - 1;
  if (hi < 0) return 0;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (start[mid] <= pos) lo = mid;
    else hi = mid - 1;
  }
  return lo;
}

// ---------- 快捷键说明（阅读器里的 ? 面板和快捷键列表共用） ----------

export const COMIC_KEYS: [string, string][] = [
  ['← / →', '翻页（从右到左阅读时 ← 是下一页）'],
  ['↑ / ↓', '上下翻页；页面超出窗口时先在页内滚动'],
  ['空格 / Shift + 空格', '下一页 / 上一页（连续滚动时滚动一屏）'],
  ['PgDn / PgUp', '下一页 / 上一页'],
  ['Home / End', '第一页 / 最后一页'],
  ['M', '切换翻页方式'],
  ['1 / 2 / 3', '单页 / 双页 / 上下连续滚动'],
  ['R', '切换阅读方向（从左到右 / 从右到左）'],
  ['Ctrl + 滚轮、Ctrl + 加号 / 减号', '放大 / 缩小'],
  ['Ctrl + 0', '还原缩放'],
  ['T', '原文 / 译文切换'],
  ['H', '显示 / 隐藏工具栏'],
  ['F', '全屏'],
  ['?', '快捷键说明'],
  ['Esc', '关闭面板 / 返回书架'],
];

export const COMIC_MOUSE: [string, string][] = [
  ['点击左右两侧', '翻页（上下翻页时点上下两侧）'],
  ['点击中间', '显示 / 隐藏工具栏'],
  ['滚轮、触摸板横向滑动', '翻页；连续滚动时平滑滚动'],
  ['按住拖动', '滑动翻页；放大后拖动查看'],
  ['双击', '放大 / 还原'],
  ['拖动底部进度条', '跳页，显示页面预览'],
];

export const NOVEL_KEYS: [string, string][] = [
  ['← / →、PgUp / PgDn', '左右翻页时翻页；上下滚动时切换章节'],
  ['空格 / Shift + 空格', '向后 / 向前一屏；自动滚动时暂停 / 继续'],
  ['↑ / ↓', '左右翻页时翻页；自动滚动时调整速度'],
  ['Home / End', '本章开头 / 结尾'],
  ['M', '切换翻页方式'],
  ['Ctrl + 加号 / 减号 / 0', '字号'],
  ['Ctrl + F', '搜索'],
  ['B', '书签'],
  ['H', '显示 / 隐藏工具栏'],
  ['F', '全屏'],
  ['?', '快捷键说明'],
  ['Esc', '关闭面板 / 返回书架'],
];
