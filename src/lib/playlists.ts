// 歌单的“⋯”菜单：播放、重命名、封面、导出、删除
import { invoke, type Playlist, type Track } from './api';
import { lib } from './library.svelte';
import { player } from './player.svelte';
import { ui, toast } from './ui.svelte';
import { ask, confirmDlg, openMenu, pickImage, RESET } from './items.svelte';

async function tracksOf(p: Playlist): Promise<Track[]> {
  const ids = await invoke<number[]>('playlist_items', { id: p.id });
  return ids.map(id => lib.byId(id)).filter((t): t is Track => !!t);
}

export async function renamePlaylist(p: Playlist) {
  const v = await ask('重命名歌单', p.name);
  if (v === null || v === RESET) return;
  if (!v.trim()) return toast('名字不能为空');
  await invoke('rename_playlist', { id: p.id, name: v.trim() });
  await lib.refreshPlaylists();
  toast('已重命名');
}

export async function deletePlaylist(p: Playlist) {
  const ok = await confirmDlg('删除歌单', `删除歌单「${p.name}」（${p.count} 首）？只删除这个列表，歌曲文件和媒体库都不受影响。`, { ok: '删除歌单', danger: true });
  if (!ok) return;
  await invoke('delete_playlist', { id: p.id });
  if (ui.playlistId === p.id) ui.playlistId = null;
  await lib.refreshPlaylists();
  toast('歌单已删除');
}

export async function setPlaylistCover(p: Playlist, reset = false) {
  const image = reset ? null : await pickImage();
  if (!reset && !image) return;
  await invoke('playlist_set_cover', { id: p.id, image });
  await lib.refreshPlaylists();
  toast(reset ? '已恢复默认封面' : '已更换封面');
}

export async function exportPlaylist(p: Playlist) {
  const { save } = await import('@tauri-apps/plugin-dialog');
  const path = await save({ defaultPath: p.name + '.m3u8', filters: [{ name: 'M3U8', extensions: ['m3u8'] }] });
  if (path) {
    await invoke('export_playlist', { id: p.id, path });
    toast('已导出');
  }
}

export function openPlaylistMenu(e: MouseEvent, p: Playlist) {
  openMenu(e, `歌单 · ${p.name}`, [
    { label: '播放', run: async () => { const t = await tracksOf(p); if (t.length) player.playList(t, 0, 'pl:' + p.id); else toast('歌单还是空的'); } },
    { label: '打开歌单', run: () => { ui.playlistId = p.id; ui.go('music'); } },
    'sep',
    { label: '重命名…', run: () => renamePlaylist(p) },
    { label: '更换封面…', run: () => setPlaylistCover(p) },
    { label: '恢复默认封面', run: () => setPlaylistCover(p, true) },
    { label: '导出 .m3u8…', run: () => exportPlaylist(p) },
    'sep',
    { label: '删除歌单…', run: () => deletePlaylist(p), danger: true },
  ]);
}
