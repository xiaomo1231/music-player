// 每日推荐：类型、后端命令的封装、显示用的小工具（后端在 src-tauri/src/daily.rs）
import { invoke } from './api';

/** sim 相似 / old 旧爱 / never 没听过 / explore 探索 */
export type SourceKind = 'sim' | 'old' | 'never' | 'explore';
export type Feedback = 'like' | 'not_song' | 'not_artist' | 'not_today';

export interface DailyItem { id: number; reason: string; kind: SourceKind | string }
export interface TopTrack { id: number; title: string; artist: string; count: number }
export interface TopArtist { name: string; count: number }
export interface DailyStats {
  yesterday: TopTrack[];
  yesterday_plays: number;
  yesterday_seconds: number;
  skipped_yesterday: number;
  top_tracks_7d: TopTrack[];
  top_artists_7d: TopArtist[];
  top_tracks_30d: TopTrack[];
  top_artists_30d: TopArtist[];
  plays_7d: number;
  skipped_7d: number;
  listen_seconds_7d: number;
  /** 近 30 天每小时的播放次数（本地时间，长 24） */
  hours: number[];
  peak_label: string | null;
  library_tracks: number;
}
export interface DailyDoc {
  /** YYYY-MM-DD（本地日期） */
  day: string;
  title: string;
  summary: string;
  tracks: DailyItem[];
  stats: DailyStats;
  ai: boolean;
  generated_at: number;
}
export interface DailyHistory { day: string; title: string; summary: string; count: number }

/** 来源徽章：样式类 + 文字（原型 badge2 b-sim / b-old / b-nev / b-exp） */
export const SOURCE: Record<string, { cls: string; label: string }> = {
  sim: { cls: 'b-sim', label: '相似' },
  old: { cls: 'b-old', label: '旧爱' },
  never: { cls: 'b-nev', label: '没听过' },
  explore: { cls: 'b-exp', label: '探索' },
};

export const daily = {
  /** 不传 day 就是今天（今天第一次请求时后端生成） */
  get: (day?: string) => invoke<DailyDoc>('daily_get', day ? { day } : {}),
  regenerate: () => invoke<DailyDoc>('daily_regenerate'),
  feedback: (fileId: number, kind: Feedback) => invoke<void>('daily_feedback', { fileId, kind }),
  history: () => invoke<DailyHistory[]>('daily_history'),
  /** name 为空时后端用“每日推荐 M-D” */
  savePlaylist: (day: string, name: string) => invoke<number>('daily_save_playlist', { day, name }),
  aiEnhance: (day: string) => invoke<DailyDoc>('daily_ai_enhance', { day }),
};

const WEEK = ['日', '一', '二', '三', '四', '五', '六'];

/** 2026-11-21 → “11 月 21 日 · 周六” */
export function dayLabel(day: string): string {
  const [y, m, d] = day.split('-').map(Number);
  if (!y || !m || !d) return day;
  return `${m} 月 ${d} 日 · 周${WEEK[new Date(y, m - 1, d).getDay()]}`;
}

/** 秒 → “1 小时 12 分” / “45 分钟” */
export function fmtDur(sec: number): string {
  const m = Math.round(sec / 60);
  return m >= 60 ? `${Math.floor(m / 60)} 小时 ${m % 60} 分` : `${m} 分钟`;
}
