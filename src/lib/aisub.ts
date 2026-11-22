// AI 字幕 / AI 服务：后端命令的类型和薄封装（只给 AiSub.svelte、AiModal.svelte 用）
import { invoke, listen } from './api';

export interface SubVideo {
  id: number; title: string; series_id: number | null; series_title: string;
  season: number | null; episode: number | null; subs: string; duration: number; path: string;
}
/** 一条字幕：原文 src，译文 zh（未翻译时为空串） */
export interface SubLine { start: number; end: number; src: string; zh: string }
export interface SubOptions { lang: string | null; target: string; output: string; glossary: string | null }
export interface JobView {
  id: number; video_id: number; title: string;
  /** queued / running / done / error / cancelled */
  status: string; step: string; progress: number; message: string; outputs: string[]; error: string | null;
}
export interface SubEstimate { minutes: number; est_tokens: number; monthly_limit: number; used_tokens: number; over_limit: boolean }
export interface SubProgress { job: number; step: string; progress: number; message: string }

export interface AiConfig {
  base_url: string; asr_model: string; chat_model: string; vision_model: string; monthly_token_limit: number;
}
export interface AiUsage {
  month: string; prompt_tokens: number; completion_tokens: number; audio_seconds: number; requests: number;
}
export interface AiState { config: AiConfig; has_key: boolean; usage: AiUsage }

/** 与后端 subtitle.rs 的步骤一一对应，界面上的六个步骤 */
export const SUB_STEPS = ['提取音轨', '检测人声', '语音识别', '断句与对齐', '翻译', '校对并输出'] as const;
export const STEP_INDEX: Record<string, number> = { extract: 0, vad: 1, asr: 2, align: 3, translate: 4, done: 5 };

export const SUB_EVENT = 'sub://progress';

/** 秒 → 0:05.3 */
export function tcLabel(s: number): string {
  const t = Math.max(0, Math.round(s * 10));
  const m = Math.floor(t / 600);
  const sec = Math.floor((t % 600) / 10);
  return `${m}:${String(sec).padStart(2, '0')}.${t % 10}`;
}

export const SUBS_LABEL: Record<string, string> = { embedded: '内封字幕', external: '外挂字幕', ai: 'AI 字幕', none: '无字幕' };

export function isLocalBase(url: string): boolean {
  return url.includes('localhost') || url.includes('127.0.0.1');
}

export const subApi = {
  listVideos: () => invoke<SubVideo[]>('sub_list_videos'),
  start: (videoId: number, options: SubOptions) => invoke<number>('sub_start', { videoId, options }),
  batch: (seriesId: number, season: number | null, options: SubOptions) =>
    invoke<number[]>('sub_batch', { seriesId, season, options }),
  cancel: (job: number) => invoke<void>('sub_cancel', { job }),
  status: () => invoke<JobView[]>('sub_status'),
  result: (job: number) => invoke<SubLine[]>('sub_result', { job }),
  save: (job: number, segments: SubLine[]) => invoke<string[]>('sub_save', { job, segments }),
  estimate: (videoId: number) => invoke<SubEstimate>('sub_estimate', { videoId }),
  realign: (videoId: number, subPath: string) => invoke<string>('sub_realign', { videoId, subPath }),
};

export const aiApi = {
  getConfig: () => invoke<AiState>('ai_get_config'),
  /** key 为 null 时保留已保存的密钥；空串表示删除 */
  saveConfig: (config: AiConfig, key: string | null) => invoke<void>('ai_save_config', { config, key }),
  test: () => invoke<string>('ai_test'),
};

export function listenSubProgress(f: (p: SubProgress) => void): Promise<() => void> {
  return listen<SubProgress>(SUB_EVENT, f);
}
