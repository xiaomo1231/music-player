// 视频与番剧模块的前端共享逻辑：缩略图目录、缩略图按需生成队列、字幕解析、类型
import { invoke, listen, inTauri, fileUrl, isAbs } from './api';
import { coverCSS, hueOf } from './util';

export interface MediaTrack { index: number; lang: string; title: string; codec: string }
export interface MediaInfo { duration: number; width: number; height: number; codec: string; audio: MediaTrack[]; subs: MediaTrack[] }
export interface SubItem { kind: 'external' | 'embedded'; path?: string; stream?: number; label: string }
export interface Cue { start: number; end: number; text: string }

class MediaState {
  /** 缩略图目录（media_thumb_dir） */
  thumbDir = $state('');
}
export const mediaState = new MediaState();
if (inTauri) {
  invoke<string>('media_thumb_dir').then(d => (mediaState.thumbDir = d)).catch(() => {});
}

/** 缩略图文件名 → 可显示的地址 */
export function thumbUrl(name: string | null | undefined): string {
  if (name && isAbs(name)) return fileUrl(name);
  return name && mediaState.thumbDir ? fileUrl(mediaState.thumbDir + '/' + name) : '';
}

/**
 * 海报背景：优先用文件夹里的海报图（series.poster 是绝对路径），其次是缩略图，最后是按名字生成的渐变
 */
export function posterBg(poster: string | null | undefined, thumb: string | null | undefined, name: string, alt = false): string {
  if (poster) return `url("${fileUrl(poster)}")`;
  const t = thumbUrl(thumb);
  if (t) return `url("${t}")`;
  return coverCSS(hueOf(name), alt);
}

/** 字幕状态角标：数据库里的 subs 字段 */
export function subBadge(subs: string): { cls: string; text: string } {
  switch (subs) {
    case 'embedded':
      return { cls: 'ok', text: '内封' };
    case 'embedded?':
      return { cls: 'ok', text: '可能内封' };
    case 'external':
      return { cls: 'ok', text: '外挂' };
    case 'ai':
      return { cls: 'ai', text: 'AI 字幕' };
    default:
      return { cls: 'no', text: '无字幕' };
  }
}

/** 集数显示：1 → "01"，1.5 → "1.5" */
export function fmtEp(n: number | null | undefined): string {
  if (n == null) return '?';
  return n % 1 === 0 ? String(n).padStart(2, '0') : String(n);
}

// ---------- 缩略图按需生成：同时最多两个 ffmpeg，结果缓存 ----------

const thumbCache = new Map<number, Promise<string | null>>();
const thumbQueue: (() => void)[] = [];
let thumbRunning = 0;

function pumpThumbs() {
  while (thumbRunning < 2 && thumbQueue.length) {
    thumbRunning++;
    thumbQueue.shift()!();
  }
}

export function requestThumb(id: number): Promise<string | null> {
  if (!inTauri) return Promise.resolve(null);
  let p = thumbCache.get(id);
  if (!p) {
    p = new Promise<string | null>(resolve => {
      thumbQueue.push(() => {
        invoke<string>('media_thumb', { id })
          .then(resolve, () => resolve(null))
          .finally(() => {
            thumbRunning--;
            pumpThumbs();
          });
      });
      pumpThumbs();
    });
    thumbCache.set(id, p);
  }
  return p;
}

/** 番剧 / 剧集卡片封面：本地海报或竖版截帧（media_series_cover） */
export interface SeriesCover { poster: string | null; frame: string | null }
const coverCache = new Map<number, Promise<SeriesCover | null>>();

export function requestSeriesCover(id: number): Promise<SeriesCover | null> {
  if (!inTauri) return Promise.resolve(null);
  let p = coverCache.get(id);
  if (!p) {
    p = new Promise<SeriesCover | null>(resolve => {
      thumbQueue.push(() => {
        invoke<SeriesCover>('media_series_cover', { id })
          .then(resolve, () => resolve(null))
          .finally(() => {
            thumbRunning--;
            pumpThumbs();
          });
      });
      pumpThumbs();
    });
    coverCache.set(id, p);
  }
  return p;
}

/** Svelte action：卡片进入可视范围时，如果还没有海报和竖版截帧就去找 / 生成 */
export function lazySeriesCover(node: HTMLElement, p: { id: number; has: boolean; oncover: (c: SeriesCover) => void }) {
  if (p.has || !inTauri || typeof IntersectionObserver === 'undefined') return {};
  let done = false;
  const io = new IntersectionObserver(
    entries => {
      if (done || !entries.some(e => e.isIntersecting)) return;
      done = true;
      io.disconnect();
      requestSeriesCover(p.id).then(c => {
        if (c && (c.poster || c.frame)) p.oncover(c);
      });
    },
    { rootMargin: '200px' },
  );
  io.observe(node);
  return { destroy: () => io.disconnect() };
}

// ---------- 联网海报（Bangumi / TMDB），后端 artmeta.rs ----------

/** 一部作品的联网匹配结果；state：auto 自动匹配 / pending 待确认 / locked 手动选的 / none 没有 */
export type ArtKind = 'series' | 'video' | 'comic';
export interface ArtInfo {
  kind: ArtKind; id: number; query?: string | null; source: string | null; remote_id: string | null; title: string | null;
  year: number | null; summary: string | null; poster: string | null; state: string;
}

class ArtState {
  series = $state<Record<number, ArtInfo>>({});
  video = $state<Record<number, ArtInfo>>({});
  comic = $state<Record<number, ArtInfo>>({});
  /** 漫画系列名 → 匹配结果 */
  comicByName = $derived(Object.fromEntries(Object.values(this.comic).filter(a => a.query).map(a => [a.query as string, a])));
}
export const art = new ArtState();
let artStarted = false;

function putArt(a: ArtInfo) {
  art[a.kind][a.id] = a;
}

async function loadComicArt() {
  const c = await invoke<ArtInfo[]>('art_list', { kind: 'comic' }).catch(() => []);
  art.comic = Object.fromEntries(c.map(a => [a.id, a]));
}

/** 读取已有的匹配结果，并让后台补全还没匹配过的（每次打开番剧 / 电影页调用，正在补全时后端直接返回） */
export async function loadArt() {
  if (!inTauri) return;
  const [s, v] = await Promise.all([
    invoke<ArtInfo[]>('art_list', { kind: 'series' }).catch(() => []),
    invoke<ArtInfo[]>('art_list', { kind: 'video' }).catch(() => []),
  ]);
  art.series = Object.fromEntries(s.map(a => [a.id, a]));
  art.video = Object.fromEntries(v.map(a => [a.id, a]));
  await loadComicArt();
  if (!artStarted) {
    artStarted = true;
    listen<{ kind: ArtKind; id: number; poster: string | null; state: string }>('art://updated', p => {
      const map = art[p.kind];
      // 新匹配上的漫画系列不知道名字，重新读一次
      if (p.kind === 'comic' && !map[p.id]) return void loadComicArt();
      putArt({ ...(map[p.id] ?? { kind: p.kind, id: p.id, source: null, remote_id: null, title: null, year: null, summary: null }), poster: p.poster, state: p.state });
    }).catch(() => {});
  }
  invoke('art_auto').catch(() => {});
}

/** 拿不准、等用户确认的 */
export const artPending = (a: ArtInfo | undefined) => a?.state === 'pending';

/** 卡片该用的海报：手动指定 / 本地海报 > 联网海报（只有自动匹配或手动选的才用） */
export function artPoster(a: ArtInfo | undefined): string | null {
  return a && (a.state === 'auto' || a.state === 'locked') ? a.poster : null;
}

/** Svelte action：电影卡片进入可视范围、又没有海报时，生成竖版截帧 */
export function lazyVideoCover(node: HTMLElement, p: { id: number; has: boolean; oncover: (frame: string) => void }) {
  if (p.has || !inTauri || typeof IntersectionObserver === 'undefined') return {};
  let done = false;
  const io = new IntersectionObserver(
    entries => {
      if (done || !entries.some(e => e.isIntersecting)) return;
      done = true;
      io.disconnect();
      thumbQueue.push(() => {
        invoke<string>('media_video_cover', { id: p.id })
          .then(p.oncover, () => {})
          .finally(() => {
            thumbRunning--;
            pumpThumbs();
          });
      });
      pumpThumbs();
    },
    { rootMargin: '200px' },
  );
  io.observe(node);
  return { destroy: () => io.disconnect() };
}

/** Svelte action：元素进入可视范围时，如果还没有缩略图就生成 */
export function lazyThumb(node: HTMLElement, p: { id: number; thumb: string | null; onthumb: (name: string) => void }) {
  if (p.thumb || !inTauri || typeof IntersectionObserver === 'undefined') return {};
  let done = false;
  const io = new IntersectionObserver(
    entries => {
      if (done || !entries.some(e => e.isIntersecting)) return;
      done = true;
      io.disconnect();
      requestThumb(p.id).then(name => {
        if (name) p.onthumb(name);
      });
    },
    { rootMargin: '200px' },
  );
  io.observe(node);
  return { destroy: () => io.disconnect() };
}

// ---------- WebVTT ----------

/** "00:01:02.345" 或 "01:02.345" → 秒 */
function vttTime(s: string): number {
  const m = s.trim().match(/^(?:(\d+):)?(\d{1,2}):(\d{1,2})[.,](\d{1,3})$/);
  if (!m) return NaN;
  return Number(m[1] || 0) * 3600 + Number(m[2]) * 60 + Number(m[3]) + Number('0.' + m[4]);
}

/** 解析 WebVTT 文本为字幕条目（去掉标签，只保留纯文字，用于渲染成普通文本） */
export function parseVtt(text: string): Cue[] {
  const out: Cue[] = [];
  const blocks = text.replace(/\r/g, '').replace(/^﻿/, '').split(/\n{2,}/);
  for (const b of blocks) {
    const lines = b.split('\n');
    const i = lines.findIndex(l => l.includes('-->'));
    if (i < 0) continue;
    const [a, rest] = lines[i].split('-->');
    const start = vttTime(a);
    const end = vttTime(rest.trim().split(/\s+/)[0] ?? '');
    if (Number.isNaN(start) || Number.isNaN(end)) continue;
    const body = lines.slice(i + 1).join('\n').replace(/<[^>]+>/g, '').trim();
    if (body) out.push({ start, end, text: body });
  }
  return out.sort((x, y) => x.start - y.start);
}

/** 字幕轨道的默认选择：优先中文，其次第一条 */
export function pickSub(subs: SubItem[]): number {
  if (!subs.length) return -1;
  const i = subs.findIndex(s => /zh|chi|chs|cht|中|简|繁|汉/i.test(s.label));
  return i >= 0 ? i : 0;
}



/** 海报角标：已看完 / 看到第几集 / 在看 / 未观看 */
export function seriesBadge(s: { episodes: number; watched: number; last_played: number | null }): string {
  if (s.episodes > 0 && s.watched >= s.episodes) return '已看完';
  if (s.watched > 0) return `看到第 ${s.watched} 集`;
  if (s.last_played != null) return '在看';
  return '未观看';
}
