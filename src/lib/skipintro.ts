// 跳过片头：每部番剧（同一 series 的所有集）单独记住片头的起止时间，存在 settings 表
// skip_intro:s<series_id>；不属于任何系列的视频按单个视频记 skip_intro:v<id>。纯逻辑，不含界面。
import { getSetting, setSetting } from './shell.svelte';

/** 没设置过时的默认行为：长于 10 分钟的视频，开头 150 秒内显示按钮，点一下前进 85 秒 */
export const DEFAULT_SKIP = 85;
const DEFAULT_WINDOW = 150;
const DEFAULT_MIN_DUR = 600;

export interface Intro {
  /** 片头开始（秒） */
  start: number;
  /** 片头结束（秒），跳过后落在这里 */
  end: number;
  /** 播到片头时自动跳过，不用点按钮 */
  auto: boolean;
}

type Owner = { id: number; series_id: number | null };

export const introKey = (v: Owner) => (v.series_id != null ? `skip_intro:s${v.series_id}` : `skip_intro:v${v.id}`);

function valid(x: unknown): x is Intro {
  const i = x as Intro;
  return !!i && typeof i.start === 'number' && typeof i.end === 'number' && i.start >= 0 && i.end > i.start;
}

export async function loadIntro(v: Owner): Promise<Intro | null> {
  const x = await getSetting<unknown>(introKey(v), null);
  return valid(x) ? { start: x.start, end: x.end, auto: !!x.auto } : null;
}

/** 传 null 恢复默认 */
export function saveIntro(v: Owner, i: Intro | null): Promise<void> {
  return setSetting(introKey(v), i);
}

/** 现在要不要显示“跳过片头”按钮 */
export function inIntro(i: Intro | null, t: number, dur: number): boolean {
  if (!i) return dur > DEFAULT_MIN_DUR && t < DEFAULT_WINDOW;
  return t >= i.start - 0.5 && t < i.end - 1;
}

/** 点“跳过片头”后跳到哪里 */
export function introTarget(i: Intro | null, t: number): number {
  return i && t < i.end ? i.end : t + DEFAULT_SKIP;
}

/** 解析 "90"、"1:30"、"1:02:03"，失败返回 null */
export function parseT(s: string): number | null {
  const p = s.trim().replace(/：/g, ':').split(':');
  if (!p.length || p.length > 3 || p.some(x => !/^\d+(\.\d+)?$/.test(x))) return null;
  return p.reduce((a, x) => a * 60 + Number(x), 0);
}
