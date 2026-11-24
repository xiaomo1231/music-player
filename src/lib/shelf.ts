// 漫画 / 小说书架按作品折叠：分组、番外识别、阅读状态等纯函数
import type { BookItem, ComicItem } from './api';

/** 作品详情页里的一话 / 一卷 */
export interface Vol {
  id: number;
  title: string;
  cover: string | null;
  /** 封面左上角的小标签（格式） */
  badge: string;
  /** 阅读进度 0–100 */
  pct: number;
  /** 读完了 */
  done: boolean;
  /** 读过（哪怕只翻了一页） */
  started: boolean;
  /** 进度文字：未读 / 第 3 / 20 页 / 已读完 */
  status: string;
  /** 番外、特典之类，单独分一组 */
  extra: boolean;
}

const natural = new Intl.Collator('zh-CN', { numeric: true, sensitivity: 'base' });
export const naturalCmp = (a: string, b: string) => natural.compare(a, b);

// 番外、特典、生日回、SP 之类。不含“外传”：很多作品本身就叫“××外传”
const EXTRA_RE = /番外|特典|特別編|特别篇|特別篇|生日|おまけ|描き下ろし|描下|附录|附錄|短篇|短編|彩页|彩頁|画集|插画|イラスト|extra|special|bonus|omake|illust|(?:^|[^a-z])sp(?:[^a-z]|$)/i;

/** 这一话 / 卷是不是番外特典（先去掉标题里的系列名，免得系列名里的字误判） */
export function isExtra(title: string, series = ''): boolean {
  const t = series && title.startsWith(series) ? title.slice(series.length) : title;
  return EXTRA_RE.test(t);
}

/** 按标题判断这部作品是按“话”还是按“卷”分的 */
export function unitOf(titles: string[]): '话' | '卷' {
  let hua = 0;
  for (const t of titles) if (/[話话回]/.test(t) && !/[卷巻冊册]|vol/i.test(t)) hua++;
  return titles.length && hua * 2 > titles.length ? '话' : '卷';
}

// ---------- 漫画 ----------

export function comicDone(c: ComicItem): boolean {
  return c.last_read != null && (c.pages <= 0 || c.page >= c.pages - 1);
}

export function comicStatus(c: ComicItem): string {
  if (c.last_read == null) return '未读';
  if (c.pages <= 0) return '已读过';
  if (c.page >= c.pages - 1) return '已读完';
  return `第 ${c.page + 1} / ${c.pages} 页`;
}

export function comicPct(c: ComicItem): number {
  if (comicDone(c)) return 100;
  return c.pages > 0 && c.last_read != null ? Math.min(100, ((c.page + 1) / c.pages) * 100) : 0;
}

export function comicFmt(c: ComicItem): string {
  if (c.format === 'pdf') return 'PDF';
  if (c.format === 'folder') return '图片';
  return c.format.toUpperCase();
}

export function comicVol(c: ComicItem): Vol {
  return {
    id: c.id,
    title: c.title,
    cover: c.cover,
    badge: comicFmt(c),
    pct: comicPct(c),
    done: comicDone(c),
    started: c.last_read != null,
    status: comicStatus(c),
    extra: isExtra(c.title, c.series),
  };
}

// ---------- 小说 ----------

const NUM = '[0-9０-９零一二三四五六七八九十百千两〇]+';
const VOL_MID = new RegExp(`^(.+?)[\\s_\\-·]*[（(\\[【]?第\\s*${NUM}\\s*[卷巻册冊部集][）)\\]】]?`);
const VOL_TAIL = new RegExp(
  `[\\s_\\-·]*[（(\\[【]?\\s*(?:vol(?:ume)?\\.?\\s*${NUM}|${NUM}\\s*[卷巻册冊部集]|[上中下](?:[卷巻册冊部集])?|${NUM})\\s*[）)\\]】]?\\s*$`,
  'i',
);

// “三体2 黑暗森林”：书名紧跟卷号，再空格接副标题
const VOL_NUM_SUB = /^(\D+?)\s*\d+\s+\S/;

/** 小说按卷拆成多个文件时的系列名（“三体2 黑暗森林”→“三体”）；看不出是分卷的返回原书名 */
export function bookSeriesKey(title: string): string {
  const t = title.trim();
  const m = t.match(VOL_MID);
  if (m && m[1].trim().length >= 2) return m[1].trim();
  const s = t.replace(VOL_TAIL, '').trim();
  if (s !== t) return s.length >= 2 ? s : t;
  const n = t.match(VOL_NUM_SUB);
  return n && n[1].trim().length >= 2 ? n[1].trim() : t;
}

export function bookProgress(b: BookItem): { label: string; pct: number; done: boolean } {
  if (b.last_read == null || b.chapters <= 0) return { label: '未读', pct: 0, done: false };
  const pct = Math.min(100, Math.round(((b.chapter + b.offset) / b.chapters) * 100));
  const done = b.chapter >= b.chapters - 1 && b.offset >= 0.98;
  return done ? { label: '已读完', pct: 100, done: true } : { label: `已读 ${pct}%`, pct, done: false };
}

export function bookVol(b: BookItem): Vol {
  const p = bookProgress(b);
  return {
    id: b.id,
    title: b.title,
    cover: b.cover,
    badge: b.format.toUpperCase(),
    pct: p.pct,
    done: p.done,
    started: b.last_read != null,
    status: p.label,
    extra: false,
  };
}

// ---------- 作品 ----------

/** 接着读哪一卷：最近读的那卷没读完就是它，读完了就是它后面第一卷没读完的；全读完返回 null */
export function nextVol<T extends { id: number }>(items: T[], done: (x: T) => boolean, lastRead: (x: T) => number | null): T | null {
  let li = -1;
  let lt = -Infinity;
  items.forEach((x, i) => {
    const t = lastRead(x);
    if (t != null && t > lt) {
      lt = t;
      li = i;
    }
  });
  if (li >= 0 && !done(items[li])) return items[li];
  for (let i = li + 1; i < items.length; i++) if (!done(items[i])) return items[i];
  return items.find(x => !done(x)) ?? null;
}

const VIEW_KEY = 'mp.shelf.view';
export function loadShelfView(): 'grid' | 'list' {
  try {
    return localStorage.getItem(VIEW_KEY) === 'list' ? 'list' : 'grid';
  } catch {
    return 'grid';
  }
}
export function saveShelfView(v: 'grid' | 'list') {
  try {
    localStorage.setItem(VIEW_KEY, v);
  } catch {
    /* 存不了就算了 */
  }
}
