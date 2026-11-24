// 条目管理：统一的“⋯”菜单、改名 / 确认对话框，以及对各类条目的操作
import { invoke, inTauri } from './api';
import { lib } from './library.svelte';
import { ui, toast } from './ui.svelte';
import { fmtSize } from './util';
import { openCoverPicker } from './media.svelte';
import { openMusicCover } from './musicart.svelte';

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

/** 批量移除后的“撤销”条 */
export interface UndoReq {
  msg: string;
  paths: string[];
}

class Items {
  menu = $state<MenuReq | null>(null);
  dialog = $state<Dialog | null>(null);
  undo = $state<UndoReq | null>(null);
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

/** 用本地图片当封面；换成了返回 true */
export async function changeCover(kind: MediaKind, id: number): Promise<boolean> {
  const p = await pickImage();
  if (!p) return false;
  const ok = await run(async () => (await invoke('item_set_cover', { kind, id, image: p }), true), '已更换封面');
  return ok === true;
}

/** 恢复默认封面：去掉手动指定的图片；有官方海报的（番剧、电视剧、电影）还会清掉手动选的海报，重新自动匹配 */
export async function resetCover(kind: MediaKind, id: number, official = false) {
  await run(async () => {
    await invoke('item_set_cover', { kind, id, image: null });
    if (official) await invoke('art_reset', { kind, id });
  }, official ? '已恢复默认封面，正在重新匹配官方海报' : '已恢复默认封面');
}

/** 歌曲恢复默认封面：去掉这一首手动指定的图，整张专辑手动选的封面也清掉，重新自动找 */
async function resetTrackCover(id: number) {
  await run(async () => {
    await invoke('item_set_cover', { kind: 'track', id, image: null });
    await invoke('music_art_reset', { track: id });
  }, '已恢复默认封面');
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
export function mediaEntries(kind: MediaKind, id: number, title: string, extra: MenuEntry[] = [], opts: { official?: boolean } = {}): MenuEntry[] {
  // 番剧 / 电视剧 / 电影：“更换封面”打开官方海报面板（面板里也能选本地图片）
  const official = opts.official ?? kind === 'series';
  return [
    ...extra,
    ...(extra.length ? (['sep'] as MenuEntry[]) : []),
    { label: '重命名…', run: () => renameItem(kind, id, title) },
    {
      label: '更换封面…',
      // 歌曲：按所在专辑联网搜封面（面板里也能用本地图片、只换这一首）
      run: () =>
        kind === 'track'
          ? openMusicCover(id, title)
          : official && (kind === 'series' || kind === 'video')
            ? openCoverPicker(kind, id, title)
            : changeCover(kind, id),
    },
    { label: '恢复默认封面', run: () => (kind === 'track' ? resetTrackCover(id) : resetCover(kind, id, official)) },
    { label: '打开所在文件夹', run: () => showInFolder(kind, id) },
    'sep',
    { label: kind === 'series' ? '从媒体库移除整部…' : '从媒体库移除…', run: () => removeItem(kind, id, title) },
    { label: kind === 'series' ? '删除整部的文件…' : '删除文件…', run: () => deleteItem(kind, id, title), danger: true },
  ];
}

// ---------- 批量 ----------

/** 多选时的量词：3 首、3 个视频、3 部、3 本、3 卷 */
export const COUNT: Record<MediaKind, (n: number) => string> = {
  track: n => `${n} 首歌曲`,
  video: n => `${n} 个视频`,
  series: n => `${n} 部`,
  book: n => `${n} 本小说`,
  comic: n => `${n} 卷漫画`,
};

let undoTimer: ReturnType<typeof setTimeout> | undefined;
function showUndo(u: UndoReq) {
  items.undo = u;
  clearTimeout(undoTimer);
  undoTimer = setTimeout(() => (items.undo = null), 10000);
}

/** 撤销批量移除：把这些路径从“已移除”名单里去掉并重新入库（播放次数、进度不会回来） */
export async function undoRemove() {
  const u = items.undo;
  if (!u) return;
  items.undo = null;
  clearTimeout(undoTimer);
  try {
    const added = await invoke<[number, string][]>('add_files', { paths: u.paths });
    await lib.refresh();
    toast(`已撤销：${added.length} 个文件重新加入媒体库`);
  } catch (e) {
    toast(String(e));
  }
}

/** 批量从媒体库移除（确认后执行，完成后显示撤销条）。返回是否已移除 */
export async function removeMany(kind: MediaKind, ids: number[]) {
  if (!ids.length) return false;
  const f = await invoke<{ files: string[] }>('item_files_many', { kind, ids });
  const what = COUNT[kind](ids.length);
  const ok = await confirmDlg(
    '从媒体库移除',
    `把选中的 ${what}${f.files.length !== ids.length ? `（共 ${f.files.length} 个文件）` : ''}从媒体库移除。硬盘上的文件不会删除，以后扫描也会跳过它们。播放次数、进度会一起清掉；移除后可以马上撤销。`,
    { ok: '移除' },
  );
  if (!ok) return false;
  try {
    const paths = await invoke<string[]>('item_remove_many', { kind, ids });
    await lib.refresh();
    showUndo({ msg: `已从媒体库移除 ${what}`, paths });
    return true;
  } catch (e) {
    toast(String(e));
    return false;
  }
}

/** 批量删除文件（移入回收站）。返回是否已删除 */
export async function deleteMany(kind: MediaKind, ids: number[]) {
  if (!ids.length) return false;
  const f = await invoke<{ files: string[]; extras: string[]; size: number }>('item_files_many', { kind, ids });
  const list = [...f.files, ...f.extras];
  const ok = await confirmDlg(
    `删除 ${COUNT[kind](ids.length)}`,
    `下面 ${list.length} 个文件（共 ${fmtSize(f.size)}${f.extras.length ? `，含 ${f.extras.length} 个同名字幕 / 歌词` : ''}）会移入回收站，同时从媒体库删除。需要时可以在回收站里还原。`,
    { list: list.length > 400 ? [...list.slice(0, 400), `……还有 ${list.length - 400} 个文件`] : list, ok: `移入回收站（${list.length} 个文件）`, danger: true },
  );
  if (!ok) return false;
  try {
    await invoke('item_delete_many', { kind, ids });
    toast(`已移入回收站：${list.length} 个文件`);
    await lib.refresh();
    return true;
  } catch (e) {
    toast(String(e));
    return false;
  }
}

/** 批量“生成 AI 字幕”：字幕页一次只接一个视频，带上第一个并提示 */
export function subtitleMany(first: number | null, n: number) {
  ui.subVideo = first;
  ui.go('aisub');
  if (n > 1) toast(`AI 字幕页一次处理一个视频，已带上第一个；同一部的整季可以在那里勾选“整季批量处理”`);
}
