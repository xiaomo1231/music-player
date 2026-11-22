// 小说阅读器与去广告共用的类型和小工具（纯逻辑，不含界面）
import { inTauri, invoke } from './api';

/** 对应 Rust adclean::Hit。下标都是按码点（字符）计算 */
export interface AdHit {
  id?: number | null;
  chapter: number;
  para: number;
  start: number;
  end: number;
  text: string;
  rule: string;
  rule_id?: number | null;
  /** hi 自动处理 / mid 待确认 */
  confidence: string;
  /** applied 已清理 / pending 待确认 / reverted 已撤销 / new 找相似的新结果 */
  status: string;
}

export interface AdRule {
  id: number;
  scope: string;
  kind: string;
  pattern: string;
  note: string;
  enabled: boolean;
  source: string;
  hits: number;
}

export interface AdEstimate {
  tokens: number;
  samples: number;
  paragraphs: number;
}

export interface Bookmark {
  chapter: number;
  offset: number;
  label: string;
  at: number;
}

export interface ReaderPrefs {
  fs: number;
  lh: number;
  font: 'song' | 'hei' | 'kai';
  theme: 'paper' | 'green' | 'night';
}

/** 面板发给阅读器的动作 */
export type AdAct =
  | { k: 'scan' }
  | { k: 'estimate' }
  | { k: 'ai' }
  | { k: 'confirm'; items: AdHit[] }
  | { k: 'cancel'; item: AdHit }
  | { k: 'revertAll' }
  | { k: 'jump'; hit: AdHit }
  | { k: 'applyCandidates'; items: AdHit[] }
  | { k: 'dismiss' }
  | { k: 'raw' }
  | { k: 'export' };

export const FONTS: Record<ReaderPrefs['font'], string> = {
  song: '"Songti SC","SimSun","Noto Serif SC",serif',
  hei: '"Microsoft YaHei UI","PingFang SC",sans-serif',
  kai: '"KaiTi","STKaiti","Kaiti SC",serif',
};

export const DEFAULT_PREFS: ReaderPrefs = { fs: 19, lh: 2, font: 'song', theme: 'paper' };

const PREFS_KEY = 'mp.reader';

/** 阅读外观只是每台机器上的偏好，存 localStorage；读写失败就用默认值 */
export function loadPrefs(): ReaderPrefs {
  try {
    const v = localStorage.getItem(PREFS_KEY);
    if (v) return { ...DEFAULT_PREFS, ...JSON.parse(v) };
  } catch {
    /* 隐私模式或被禁用时忽略 */
  }
  return { ...DEFAULT_PREFS };
}

export function savePrefs(p: ReaderPrefs) {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(p));
  } catch {
    /* 忽略 */
  }
}

/** 按码点切分（和 Rust 的 chars() 一致） */
export const cps = (s: string): string[] => Array.from(s);

export interface Seg {
  t: string;
  /** 0 正常 / 1 待确认 / 2 已清理 */
  s: number;
}

/**
 * 把一段文字按状态切成几段。hits 的区间按码点计，state 取最大值（已清理 > 待确认）
 */
export function segments(text: string, hits: { start: number; end: number; state: number }[]): { segs: Seg[]; allGone: boolean } {
  const chars = cps(text);
  const st = new Uint8Array(chars.length);
  for (const h of hits) {
    for (let i = Math.max(0, h.start); i < Math.min(chars.length, h.end); i++) st[i] = Math.max(st[i], h.state);
  }
  const segs: Seg[] = [];
  let cur = '';
  let cs = -1;
  chars.forEach((c, i) => {
    if (st[i] !== cs) {
      if (cur) segs.push({ t: cur, s: cs });
      cur = '';
      cs = st[i];
    }
    cur += c;
  });
  if (cur) segs.push({ t: cur, s: cs });
  const allGone = chars.length > 0 && st.every(x => x === 2);
  return { segs, allGone };
}

/** 段落里 (node, off) 之前的字符数（按码点），用来把选区换算成段内下标 */
export function offsetIn(root: HTMLElement, node: Node, off: number): number {
  const r = document.createRange();
  r.selectNodeContents(root);
  r.setEnd(node, off);
  return cps(r.toString()).length;
}

/** 判断一条命中是否还对得上原文（文件变了就不套用） */
export function hitMatches(paraText: string, h: { start: number; end: number; text: string }): boolean {
  return cps(paraText).slice(h.start, h.end).join('') === h.text;
}

export async function loadBookmarks(id: number): Promise<Bookmark[]> {
  if (!inTauri) return [];
  try {
    const v = await invoke<Bookmark[] | null>('get_setting', { key: `bookmarks:${id}` });
    return Array.isArray(v) ? v : [];
  } catch {
    return [];
  }
}

export async function saveBookmarks(id: number, list: Bookmark[]) {
  if (!inTauri) return;
  await invoke('set_setting', { key: `bookmarks:${id}`, value: list });
}
