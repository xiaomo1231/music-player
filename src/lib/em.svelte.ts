// .em 规则文件：类型、命令、导入确认框的状态
import { invoke, inTauri, listen, bumpComicPages } from './api';
import { toast } from './ui.svelte';

export type EmKind = 'comic' | 'source' | 'clean' | 'organize';

export interface EmRule {
  id: string;
  kind: EmKind;
  kind_label: string;
  name: string;
  version: string;
  author: string;
  note: string;
  hosts: string[];
  enabled: boolean;
  path: string;
  installed_at: number;
  updated_at: number;
  /** 有问题时的说明（规则不会生效） */
  problem: string;
}

export interface EmIssue {
  line: number;
  msg: string;
}

export interface EmPreview {
  ok: boolean;
  issues: EmIssue[];
  id: string;
  kind: EmKind;
  kind_label: string;
  name: string;
  version: string;
  author: string;
  note: string;
  hosts: string[];
  lan_hosts: string[];
  login: 'none' | 'optional' | 'required';
  summary: string[];
  installed_version: string | null;
  new_hosts: string[];
}

export const EM_KINDS: { id: EmKind; name: string }[] = [
  { id: 'comic', name: '漫画排版' },
  { id: 'source', name: '下载源' },
  { id: 'clean', name: '小说排版' },
  { id: 'organize', name: '整理文件' },
];

export const isEmFile = (p: string) => /\.em$/i.test(p);

export const issueText = (i: EmIssue) => (i.line > 0 ? `第 ${i.line} 行：${i.msg}` : i.msg);

class EmState {
  /** 正在确认导入的文件（队列，一个一个确认） */
  pending = $state<string[]>([]);
  /** 规则有变化时 +1（规则列表、阅读器等据此刷新） */
  version = $state(0);
}
export const em = new EmState();

export const emList = () => (inTauri ? invoke<EmRule[]>('em_list') : Promise.resolve([] as EmRule[]));
export const emInspect = (path: string) => invoke<EmPreview>('em_inspect', { path });
export const emInstall = (path: string) => invoke<EmRule>('em_install', { path });
export const emSetEnabled = (id: string, enabled: boolean) => invoke<void>('em_set_enabled', { id, enabled });
export const emDelete = (id: string) => invoke<void>('em_delete', { id });
export const emExport = (id: string, path: string) => invoke<void>('em_export', { id, path });
export const emTest = <T = unknown>(id: string, arg: unknown = null) => invoke<T>('em_test', { id, arg });

/** 打开、拖入或在规则页选择了 .em 文件：进入导入确认 */
export function importEm(paths: string[]) {
  const add = paths.filter(isEmFile).filter(p => !em.pending.includes(p));
  if (add.length) em.pending = [...em.pending, ...add];
}

/** 在规则页点“导入…” */
export async function pickEm() {
  if (!inTauri) return;
  const { open } = await import('@tauri-apps/plugin-dialog');
  const p = await open({ multiple: true, filters: [{ name: '规则文件', extensions: ['em'] }] });
  if (!p) return;
  importEm(Array.isArray(p) ? p : [p]);
}

/** 把 .em 文件从一批路径里分出来；返回剩下的路径 */
export function takeEm(paths: string[]): string[] {
  const ems = paths.filter(isEmFile);
  if (ems.length) importEm(ems);
  return paths.filter(p => !isEmFile(p));
}

let started = false;
/** 启动时调用一次：监听后端的规则变化 */
export function startEm() {
  if (started || !inTauri) return;
  started = true;
  void listen<string>('em://changed', kind => {
    // 漫画排版规则变了：页面图片地址换掉，不用缓存的旧图
    if (kind === 'comic') bumpComicPages();
    em.version++;
  });
}

export async function exportEm(r: EmRule) {
  const { save } = await import('@tauri-apps/plugin-dialog');
  const p = await save({ defaultPath: `${r.id}.em`, filters: [{ name: '规则文件', extensions: ['em'] }] });
  if (!p) return;
  try {
    await emExport(r.id, p);
    toast('已导出');
  } catch (e) {
    toast(String(e));
  }
}

export const emOpen = (id: string) => invoke<void>('em_open', { id });
