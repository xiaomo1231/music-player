// 搜索页 B 站结果的下载选项：音视频 / 仅音频 / 仅视频 / 仅元数据、清晰度、字幕、弹幕、封面。
// 选项编进下载源任务地址的档位（q），后端 bili::Choice 解析：`m=a;h=1080;s=1;d=0;c=1`
import { withQuality, type Hit } from './fetch';

export type BiliMode = 'av' | 'a' | 'v' | 'meta';

export interface BiliChoice {
  mode: BiliMode;
  /** 清晰度上限（画面高度）：'' 按设置，'0' 最高 */
  height: string;
  subs: boolean;
  danmaku: boolean;
  cover: boolean;
}

export const BILI_MODES: { key: BiliMode; label: string; hint: string }[] = [
  { key: 'av', label: '音视频', hint: '画面和声音合成一个 MP4，存进视频库' },
  { key: 'a', label: '仅音频', hint: '只要声音（M4A，带封面和标签），存进音乐库' },
  { key: 'v', label: '仅视频', hint: '只要画面，没有声音' },
  { key: 'meta', label: '仅元数据', hint: '标题、UP 主、简介、标签等信息和封面，不下载音视频' },
];

export const BILI_HEIGHTS: [string, string][] = [
  ['', '按设置'],
  ['0', '最高'],
  ['1080', '1080P'],
  ['720', '720P'],
  ['480', '480P'],
];

/** 这条结果来自内置的 B 站下载源 */
export const isBiliHit = (h: Hit | undefined): boolean => !!h && h.kind === 'provider' && h.url.startsWith('provider://bilibili/');

/** 这种内容要不要选清晰度、字幕弹幕 */
export const hasPicture = (m: BiliMode) => m === 'av' || m === 'v';
export const hasSubs = (m: BiliMode) => m !== 'a';

export function choiceKey(c: BiliChoice): string {
  const parts = [`m=${c.mode}`];
  if (hasPicture(c.mode) && c.height !== '') parts.push(`h=${c.height}`);
  if (hasSubs(c.mode)) parts.push(`s=${c.subs ? 1 : 0}`, `d=${c.danmaku ? 1 : 0}`);
  if (c.mode !== 'meta') parts.push(`c=${c.cover ? 1 : 0}`);
  return parts.join(';');
}

export function choiceLabel(c: BiliChoice): string {
  const parts = [BILI_MODES.find(m => m.key === c.mode)?.label ?? '音视频'];
  if (hasPicture(c.mode)) parts.push(BILI_HEIGHTS.find(h => h[0] === c.height)?.[1] ?? `${c.height}P`);
  if (hasSubs(c.mode)) {
    if (c.subs) parts.push('字幕');
    if (c.danmaku) parts.push('弹幕');
  }
  if (c.cover && c.mode !== 'meta') parts.push('封面');
  return parts.join(' · ');
}

/** 仅音频存进音乐库；其他内容在“音乐”分类里搜到的也存进视频库 */
export function choiceMediaKind(c: BiliChoice, mk: string): string {
  if (c.mode === 'a') return 'music';
  return mk === 'music' ? 'video' : mk;
}

/** 把选项套到一条结果上：改档位，下载列表里的标题也换成所选内容 */
export function applyChoice(h: Hit, c: BiliChoice): Hit {
  const out = withQuality(h, choiceKey(c));
  const title = h.title.replace(/ \[[^\]]*\]$/, '');
  return { ...out, title: `${title} [${choiceLabel(c)}]`, size: null };
}

const storeKey = (mk: string) => `mp.bili.choice.${mk === 'music' ? 'music' : 'video'}`;

/** 上次选的（音乐分类和其他分类各记一份）；没有时按分类给默认值，字幕弹幕跟设置走 */
export function loadChoice(mk: string, subs = true, danmaku = true): BiliChoice {
  const def: BiliChoice = { mode: mk === 'music' ? 'a' : 'av', height: '', subs, danmaku, cover: false };
  try {
    const raw = localStorage.getItem(storeKey(mk));
    if (raw) {
      const v = JSON.parse(raw) as Partial<BiliChoice>;
      if (v && BILI_MODES.some(m => m.key === v.mode)) return { ...def, ...v };
    }
  } catch {
    /* 读不到就用默认 */
  }
  return def;
}

export function saveChoice(mk: string, c: BiliChoice) {
  try {
    localStorage.setItem(storeKey(mk), JSON.stringify(c));
  } catch {
    /* 存不了也不影响这次下载 */
  }
}

export function hasSavedChoice(mk: string): boolean {
  try {
    return !!localStorage.getItem(storeKey(mk));
  } catch {
    return false;
  }
}
