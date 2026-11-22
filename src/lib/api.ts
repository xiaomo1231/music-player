// 与 Rust 后端通信。浏览器里直接打开（没有 Tauri）时返回空数据，方便调界面
import { invoke as tauriInvoke, convertFileSrc } from '@tauri-apps/api/core';
import { listen as tauriListen, type UnlistenFn } from '@tauri-apps/api/event';

export const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) throw new Error('不在桌面程序中运行：' + cmd);
  return tauriInvoke<T>(cmd, args);
}

export async function listen<T>(ev: string, f: (p: T) => void): Promise<UnlistenFn> {
  if (!inTauri) return () => {};
  return tauriListen<T>(ev, e => f(e.payload));
}

export const fileUrl = (p: string) => (inTauri ? convertFileSrc(p) : p);

let coverDir = '';
export async function initPaths() {
  if (inTauri) coverDir = await invoke<string>('cover_dir');
}
export const coverUrl = (name?: string | null) => (name ? fileUrl(coverDir + '/' + name) : '');
export const comicPage = (id: number, page: number) => (inTauri ? `http://mpc.localhost/${id}/${page}` : '');

export interface Track {
  id: number; path: string; title: string; artist: string; album: string; album_artist: string;
  track_no: number | null; disc_no: number | null; year: number | null; genre: string; duration: number;
  format: string; lossless: boolean; bitrate: number | null; sample_rate: number | null; bit_depth: number | null;
  cover: string | null; hue: number | null; play_count: number; skip_count: number; last_played: number | null;
  favorite: boolean; added_at: number; encrypted: boolean; size: number;
}
export interface Video {
  id: number; path: string; title: string; series_id: number | null; season: number | null; episode: number | null;
  group: string | null; resolution: string | null; duration: number; position: number; watched: boolean;
  last_played: number | null; subs: string; thumb: string | null; size: number; added_at: number;
}
export interface Series {
  id: number; title: string; kind: string; poster: string | null; episodes: number; watched: number; seasons: number;
  missing_subs: number; last_played: number | null; next_id: number | null; next_label: string | null; thumb: string | null;
}
export interface BookItem {
  id: number; path: string; title: string; author: string; format: string; chapter: number; chapters: number;
  offset: number; cover: string | null; last_read: number | null; size: number;
}
export interface ComicItem {
  id: number; path: string; title: string; series: string; format: string; pages: number; page: number;
  cover: string | null; last_read: number | null;
}
export interface Playlist { id: number; name: string; count: number; covers: string[] }
export interface Hit { id: number; kind: string; title: string; sub: string; cover: string | null; series_id: number | null }
export interface LyricLine { t: number; text: string; words: [number, string][] }
export interface Chapter { title: string; paras: string[] }
export interface Book { title: string; author: string; encoding: string; chapters: Chapter[] }
export interface Root { id: number; path: string; watch: boolean }
export interface Drive { root: string; label: string; total: number; free: number }
export interface ScanProgress {
  phase: string; path: string; found: number[]; total: number; done: number; added: number; updated: number;
  missing: number; recent: [string, string][];
}
export interface AppInfo { version: string; engine: 'web' | 'mpv'; data_dir: string; cover_dir: string }
export interface Stats { music: number; video: number; anime: number; manga: number; novel: number }
