// 译文排版：按气泡大小自动选字号、换行，横排或竖排。页面叠加层（DOM）和导出（canvas）共用，结果一致。
export const FONT = '"Microsoft YaHei UI","Microsoft YaHei","PingFang SC","Noto Sans SC",sans-serif';
export const MIN_FONT = 9;
const MAX_FONT = 48;
/** 文字区域四周留的边距（占气泡宽高的比例） */
const PAD = 0.06;
/** 横排行高 / 竖排列宽 相对字号的倍数 */
const LINE_H = 1.25;
const COL_W = 1.2;

export type Measure = (text: string, size: number) => number;

export interface Fit {
  size: number;
  /** 横排：每行一句；竖排：每列一段（按阅读顺序，从右到左排列） */
  lines: string[];
}

let mctx: CanvasRenderingContext2D | null | undefined;

/** 按字体测量文字宽度（像素）。没有 canvas 时按字数估算。 */
export function measureText(text: string, size: number): number {
  if (mctx === undefined) mctx = typeof document === 'undefined' ? null : document.createElement('canvas').getContext('2d');
  if (!mctx) return Array.from(text).length * size;
  mctx.font = `${size}px ${FONT}`;
  return mctx.measureText(text).width;
}

/** 日文汉字、假名、全角符号等按“一字一个”处理，其余按词处理 */
function isWide(ch: string): boolean {
  return ch.charCodeAt(0) > 0x2e7f;
}

function tokenize(t: string): string[] {
  const out: string[] = [];
  let word = '';
  const flush = () => {
    if (word) out.push(word);
    word = '';
  };
  for (const ch of t) {
    if (isWide(ch)) {
      flush();
      out.push(ch);
    } else if (ch === ' ') {
      word += ' ';
      flush();
    } else {
      word += ch;
    }
  }
  flush();
  return out;
}

/** 横排换行：贪心地把词放进一行，放不下就换行。 */
export function wrapH(text: string, width: number, size: number, measure: Measure): string[] {
  const lines: string[] = [];
  let cur = '';
  for (const tok of tokenize(text)) {
    const cand = cur + tok;
    if (cur.trim() && measure(cand.trimEnd(), size) > width) {
      lines.push(cur.trimEnd());
      cur = tok.trimStart();
    } else {
      cur = cand;
    }
  }
  if (cur.trim()) lines.push(cur.trimEnd());
  return lines;
}

/** 竖排分列：每列放 perCol 个字 */
export function chunkV(text: string, height: number, size: number): string[] {
  const chars = Array.from(text.replace(/\s+/g, ''));
  const perCol = Math.max(1, Math.floor(height / size));
  const cols: string[] = [];
  for (let i = 0; i < chars.length; i += perCol) cols.push(chars.slice(i, i + perCol).join(''));
  return cols;
}

/** 在 w×h（像素）的气泡里找能放下的最大字号 */
export function fitText(text: string, w: number, h: number, vertical: boolean, measure: Measure = measureText): Fit {
  const t = text.replace(/\s*\n\s*/g, ' ').trim();
  if (!t || !(w > 0) || !(h > 0)) return { size: MIN_FONT, lines: [] };
  const iw = w * (1 - 2 * PAD);
  const ih = h * (1 - 2 * PAD);
  const start = Math.min(MAX_FONT, Math.floor(vertical ? iw / COL_W : ih));
  for (let size = Math.max(start, MIN_FONT); size >= MIN_FONT; size--) {
    if (vertical) {
      const cols = chunkV(t, ih, size);
      if (cols.length * size * COL_W <= iw) return { size, lines: cols };
    } else {
      const lines = wrapH(t, iw, size, measure);
      if (lines.length * size * LINE_H <= ih && lines.every(l => measure(l, size) <= iw)) return { size, lines };
    }
  }
  // 实在放不下：用最小字号，超出部分由容器裁掉
  return { size: MIN_FONT, lines: vertical ? chunkV(t, ih, MIN_FONT) : wrapH(t, iw, MIN_FONT, measure) };
}

/** 底色太深时字用白色，否则用黑色 */
export function textColor(bg?: string | null): string {
  const m = /^#?([0-9a-f]{6})$/i.exec(bg ?? '#ffffff');
  if (!m) return '#111';
  const n = parseInt(m[1], 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  return 0.299 * r + 0.587 * g + 0.114 * b > 140 ? '#111' : '#fff';
}

export const LINE_HEIGHT = LINE_H;
export const COLUMN_WIDTH = COL_W;
