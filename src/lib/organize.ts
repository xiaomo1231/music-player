// 整理文件：类型、后端命令的封装（后端在 src-tauri/src/organize.rs）
import { invoke } from './api';

export interface OrgExtra { src: string; dst: string }
export interface OrgItem {
  src: string;
  /** 空字符串 = 原地不动 */
  dst: string;
  kind: string;
  /** hi 把握高 / mid 需要看一眼 / low 把握低 */
  confidence: string;
  reason: string;
  extras: OrgExtra[];
  checked: boolean;
}
export interface OrgBatch { id: number; created_at: number; mode: string; undone: boolean; count: number }
export interface OrgFailed { path: string; error: string }
export interface OrgReport { batch_id: number | null; done: number; failed: OrgFailed[] }
export interface OrgUndo { reverted: number; failed: OrgFailed[]; finished: boolean }
export interface OrgProgress { done: number; total: number; current: string; phase: string }
export interface OrgEstimate { files: number; tokens: number }

export const MODES: [string, string][] = [
  ['move', '移动（同盘瞬间完成）'],
  ['copy', '复制（保留原文件）'],
  ['hardlink', '硬链接（不影响做种）'],
  ['library', '只在软件里整理（硬盘不动）'],
];

export const MODE_LABEL: Record<string, string> = Object.fromEntries(MODES);

export const CONF: Record<string, { cls: string; label: string }> = {
  hi: { cls: 'ok', label: '把握高' },
  mid: { cls: 'no', label: '需要看一眼' },
  low: { cls: '', label: '把握低' },
};

export const org = {
  scan: (roots: string[], types: string, targetRoot: string) => invoke<OrgItem[]>('org_scan', { roots, types, targetRoot }),
  estimate: (items: OrgItem[]) => invoke<OrgEstimate>('org_estimate', { items }),
  aiIdentify: (items: OrgItem[], targetRoot: string) => invoke<OrgItem[]>('org_ai_identify', { items, targetRoot }),
  execute: (items: OrgItem[], mode: string, cleanupEmpty: boolean) => invoke<OrgReport>('org_execute', { items, mode, cleanupEmpty }),
  undo: (batchId: number) => invoke<OrgUndo>('org_undo', { batchId }),
  batches: () => invoke<OrgBatch[]>('org_batches'),
};

/** 这次会动的文件数（勾选的主文件 + 要跟着走的配套文件） */
export function planCount(items: OrgItem[], withExtras: boolean): number {
  let n = 0;
  for (const it of items) {
    if (!it.checked || !it.dst) continue;
    n += 1;
    if (withExtras) n += it.extras.filter(x => x.dst).length;
  }
  return n;
}
