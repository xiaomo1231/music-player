// 条目管理：统一的“⋯”菜单、改名 / 确认对话框，以及对各类条目的操作
import { invoke, inTauri } from './api';
import { lib } from './library.svelte';
import { toast } from './ui.svelte';
import { fmtSize } from './util';

export type MediaKind = 'track' | 'video' | 'series' | 'book' | 'comic';
export type MenuEntry = { label: string; run: () => unknown; danger?: boolean } | 'sep';

export interface MenuReq {
  x: number;
  y: number;
  title: string;
  entries: MenuEntry[];
}

export type Dialog =
  | { type: 'prompt'; title: string; desc?: string; value: string; placeholder?: string; reset?: string; resolve: (v: string | null | typeof RESET) => void }
  | { type: 'confirm'; title: string; desc: string; list?: string[]; ok: string; danger?: boolean; resolve: (ok: boolean) => void };

export const RESET = Symbol('reset');

class Items {
  menu = $state<MenuReq | null>(null);
  dialog = $state<Dialog | null>(null);
}
export const items = new Items();

/** 在鼠标位置或按钮下方弹出菜单 */
export function openMenu(e: MouseEvent, title: string, entries: MenuEntry[]) {
  e.preventDefault();
  e.stopPropagation();
  const el = e.currentTarget as HTMLElement | null;
  let { clientX: x, clientY: y } = e;
  // 键盘或按钮触发时贴着按钮
  if (e.type === 'click' && el?.getBoundingClientRect) {
    const r = el.getBoundingClientRect();
    x = r.right;
    y = r.bottom + 4;
  }
  items.menu = { x, y, title, entries };
}

export function ask(title: string, value: string, opts: { desc?: string; placeholder?: string; reset?: string } = {}) {
  return new Promise<string | null | typeof RESET>(resolve => {
    items.dialog = { type: 'prompt', title, value, ...opts, resolve };
  });
}

export function confirmDlg(title: string, desc: string, opts: { list?: string[]; ok?: string; danger?: boolean } = {}) {
  return new Promise<boolean>(resolve => {
    items.dialog = { type: 'confirm', title, desc, list: opts.list, ok: opts.ok ?? '确定', danger: opts.danger, resolve };
  });
}

export async function pickImage(): Promise<string | null> {
  if (!inTauri) return null;
  const { open } = await import('@tauri-apps/plugin-dialog');
  const p = await open({ multiple: false, filters: [{ name: '图片', extensions: ['jpg', 'jpeg', 'png', 'webp', 'bmp', 'gif'] }] });
  return typeof p === 'string' ? p : null;
}

const NOUN: Record<MediaKind, string> = { track: '歌曲', video: '视频', series: '整部', book: '小说', comic: '漫画' };

async function run<T>(f: () => Promise<T>, ok: string) {
  try {
    const r = await f();
    toast(ok);
    await lib.refresh();
    return r;
  } catch (e) {
    toast(String(e));
    return undefined;
  }
}

export async function renameItem(kind: MediaKind, id: number, current: string) {
  const v = await ask('重命名', current, { desc: '只改软件里显示的名字，硬盘上的文件名不变；重新扫描也不会被覆盖。', reset: '恢复原名' });
  if (v === null) return;
  const name = v === RESET ? null : v.trim();
  if (name === '') return toast('名字不能为空');
  await run(() => invoke('item_rename', { kind, id, name }), name ? '已重命名' : '已恢复原名');
}

export async function changeCover(kind: MediaKind, id: number) {
  const p = await pickImage();
  if (p) await run(() => invoke('item_set_cover', { kind, id, image: p }), '已更换封面');
}

export async function resetCover(kind: MediaKind, id: number) {
  await run(() => invoke('item_set_cover', { kind, id, image: null }), '已恢复默认封面');
}

export async function showInFolder(kind: MediaKind, id: number) {
  const f = await invoke<{ files: string[] }>('item_files', { kind, id });
  if (f.files[0]) invoke('show_in_folder', { path: f.files[0] });
}

export async function removeItem(kind: MediaKind, id: number, title: string) {
  const f = await invoke<{ files: string[] }>('item_files', { kind, id });
  const many = f.files.length > 1;
  const ok = await confirmDlg(
    '从媒体库移除',
    `把「${title}」${many ? `（共 ${f.files.length} 个文件）` : ''}从媒体库移除。硬盘上的文件不会删除，以后扫描也会跳过它们；想加回来可以把文件拖进窗口。播放次数、进度会一起清掉。`,
    { ok: '移除' },
  );
  if (ok) await run(() => invoke('item_remove', { kind, id }), `已从媒体库移除${many ? ` ${f.files.length} 个文件` : ''}`);
}

export async function deleteItem(kind: MediaKind, id: number, title: string) {
  const f = await invoke<{ files: string[]; extras: string[]; size: number }>('item_files', { kind, id });
  const list = [...f.files, ...f.extras];
  const ok = await confirmDlg(
    `删除${NOUN[kind]}文件`,
    `下面 ${list.length} 个文件（共 ${fmtSize(f.size)}${f.extras.length ? `，含 ${f.extras.length} 个同名字幕 / 歌词` : ''}）会移入回收站，「${title}」同时从媒体库删除。需要时可以在回收站里还原。`,
    { list, ok: '移入回收站', danger: true },
  );
  if (ok) await run(() => invoke('item_delete', { kind, id }), `已移入回收站：${list.length} 个文件`);
}

/** 媒体条目的通用菜单项：改名、封面、文件夹、移除、删除 */
export function mediaEntries(kind: MediaKind, id: number, title: string, extra: MenuEntry[] = []): MenuEntry[] {
  return [
    ...extra,
    ...(extra.length ? (['sep'] as MenuEntry[]) : []),
    { label: '重命名…', run: () => renameItem(kind, id, title) },
    { label: '更换封面…', run: () => changeCover(kind, id) },
    { label: '恢复默认封面', run: () => resetCover(kind, id) },
    { label: '打开所在文件夹', run: () => showInFolder(kind, id) },
    'sep',
    { label: kind === 'series' ? '从媒体库移除整部…' : '从媒体库移除…', run: () => removeItem(kind, id, title) },
    { label: kind === 'series' ? '删除整部的文件…' : '删除文件…', run: () => deleteItem(kind, id, title), danger: true },
  ];
}
