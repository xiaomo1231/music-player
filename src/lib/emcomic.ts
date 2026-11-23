// 漫画排版规则（.em kind: comic）：阅读器、书架、规则测试面板共用
import { invoke, inTauri } from './api';
import type { ComicPrefs } from './readerprefs';

export interface ComicRuleRead {
  flow?: ComicPrefs['flow'];
  dir?: ComicPrefs['dir'];
  spread?: ComicPrefs['spread'];
  cover_alone?: boolean;
  fit?: ComicPrefs['fit'];
  gap?: number;
}

export interface ComicView {
  rule: { id: string; name: string } | null;
  read: ComicRuleRead | null;
  /** 排版后的页数 */
  pages: number;
  /** 规则改动了页面（拆、裁、跳过） */
  processed: boolean;
  ver: string;
  skipped: number;
  /** 这本是按这个规则导出的，页面不再处理 */
  marker: boolean;
}

export interface ComicExportReport {
  outputs: string[];
  failed: string[];
}

export interface ComicExportProgress {
  done: number;
  total: number;
  current: string;
  page: number;
  pages: number;
}

export const comicView = (id: number) => (inTauri ? invoke<ComicView>('em_comic_view', { id }) : Promise.resolve(null));

export const comicExport = (ids: number[], replace: boolean) => invoke<ComicExportReport>('em_comic_export', { ids, replace });

/** 规则里的阅读排版套到偏好上（只在这本书没单独设置过时用） */
export function applyRuleRead(p: ComicPrefs, r: ComicRuleRead): ComicPrefs {
  const o = { ...p };
  if (r.flow) o.flow = r.flow;
  if (r.dir) o.dir = r.dir;
  if (r.spread) o.spread = r.spread;
  if (typeof r.cover_alone === 'boolean') o.coverAlone = r.cover_alone;
  if (r.fit) o.fit = r.fit;
  if (typeof r.gap === 'number') o.gap = r.gap;
  return o;
}

// 规则测试面板
export interface ComicTestPage {
  i: number;
  src: number;
  name: string;
  half: 'full' | 'left' | 'right';
  trim: boolean;
}

export interface ComicTestResult {
  title: string;
  matched: boolean;
  before: number;
  after: number;
  split: number;
  trimmed: number;
  skipped: number;
  processed: boolean;
  marker: boolean;
  ver: string;
  pages: ComicTestPage[];
  skipped_pages: { src: number; name: string; why: string }[];
}

export interface ComicTestBook {
  id: number;
  title: string;
  series: string;
  matched: boolean;
}
