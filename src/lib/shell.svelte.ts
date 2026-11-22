// 窗口外壳的前端部分：托盘 / 热键命令、防休眠、桌面歌词推送、壁纸模式、外壳相关的设置读写。
// 集成：main.ts 里（非桌面歌词窗口）挂载 App 之后调用一次 initShell()，见 docs/integration/shell.md。
import { emit } from '@tauri-apps/api/event';
import { invoke, inTauri, listen, type Track } from './api';
import { lib } from './library.svelte';
import { player } from './player.svelte';
import { lyricsfx } from './lyricsfx.svelte';
import { ui, toast } from './ui.svelte';

/** 桌面歌词窗口加载的地址是 index.html#lyrics */
export const isLyricsWindow = (): boolean => typeof location !== 'undefined' && location.hash === '#lyrics';

export interface HotkeyStatus {
  action: string;
  accel: string;
  enabled: boolean;
  error?: string;
}
export interface HotkeyCfg {
  action: string;
  accel: string;
  enabled: boolean;
}
export interface AutostartInfo {
  enabled: boolean;
  minimized: boolean;
}
export interface CacheInfo {
  thumbs: number;
  covers: number;
}
interface OpenPayload {
  paths: string[];
  addOnly: boolean;
}

export const HOTKEY_LABEL: Record<string, string> = {
  play_pause: '播放 / 暂停（多媒体键）',
  media_next: '下一首（多媒体键）',
  media_prev: '上一首（多媒体键）',
  next: '下一首',
  prev: '上一首',
  vol_up: '音量加',
  vol_down: '音量减',
  window: '显示 / 隐藏主窗口',
  fav: '收藏当前歌曲',
  lyrics: '显示 / 隐藏桌面歌词',
};

/** 设置读写（settings 表，JSON）。浏览器里直接打开时返回默认值 */
export async function getSetting<T>(key: string, d: T): Promise<T> {
  if (!inTauri) return d;
  try {
    const v = await invoke<T | null>('get_setting', { key });
    return v ?? d;
  } catch {
    return d;
  }
}

export async function setSetting(key: string, value: unknown): Promise<void> {
  if (!inTauri) return;
  try {
    await invoke('set_setting', { key, value });
  } catch (e) {
    toast('保存设置失败：' + String(e));
  }
}

/** 把键盘事件转成热键字符串，例如 "Ctrl+Alt+KeyP"；只按修饰键时返回 null */
export function accelFromEvent(e: KeyboardEvent): string | null {
  if (['Control', 'Alt', 'Shift', 'Meta', 'OS'].includes(e.key) || !e.code) return null;
  const mods: string[] = [];
  if (e.ctrlKey) mods.push('Ctrl');
  if (e.altKey) mods.push('Alt');
  if (e.shiftKey) mods.push('Shift');
  if (e.metaKey) mods.push('Win');
  return [...mods, e.code].join('+');
}

// ---------------------------------------------------------------- 热键 / 开机启动 / 缓存 / 资源管理器

export const loadHotkeys = () => invoke<HotkeyStatus[]>('shell_hotkeys');
export const saveHotkeys = (list: HotkeyCfg[]) => invoke<HotkeyStatus[]>('shell_set_hotkeys', { list });
export const resetHotkeys = () => invoke<HotkeyStatus[]>('shell_hotkeys_reset');

export const autostartGet = () => invoke<AutostartInfo>('shell_autostart_get');
export const autostartSet = (enabled: boolean, minimized: boolean) => invoke<void>('shell_autostart_set', { enabled, minimized });

export const cacheSize = () => invoke<CacheInfo>('shell_cache_size');
export const cacheClear = (covers: boolean) => invoke<number>('shell_cache_clear', { covers });

export async function setExplorerMenu(on: boolean): Promise<void> {
  await invoke('shell_explorer_menu', { on });
  await setSetting('explorer_menu', on);
}

/** 壁纸模式：开启时同时进入沉浸模式；失败时抛出错误（调用方用 toast 显示） */
export async function setWallpaper(on: boolean): Promise<void> {
  if (on) {
    await ui.setImmersive(true);
    try {
      await invoke('shell_wallpaper', { on: true });
    } catch (e) {
      ui.setImmersive(false);
      throw e;
    }
  } else {
    await invoke('shell_wallpaper', { on: false });
    ui.setImmersive(false);
  }
}

// ---------------------------------------------------------------- 桌面歌词

let lyricsOn = false;
let lastLine = '';

/** 当前歌词行（没有歌词时为空）；有译文且开启“显示翻译”时第二行是译文 */
function currentLine(): string {
  const i = player.lyricIndex();
  const l = i >= 0 ? player.lyrics[i] : undefined;
  if (!l) return '';
  return lyricsfx.cfg.tr && l.tr ? `${l.text}\n${l.tr}` : l.text;
}

function pushLine(force = false) {
  const t = currentLine();
  if (!force && t === lastLine) return;
  lastLine = t;
  if (inTauri) emit('lyrics://line', t).catch(() => {});
}

export async function setDesktopLyrics(on: boolean): Promise<void> {
  await invoke('shell_desktop_lyrics', { on });
  lyricsOn = on;
  await setSetting('desktop_lyrics', on);
  if (on) pushLine(true);
}

// ---------------------------------------------------------------- 启动文件

async function openPaths(paths: string[], addOnly: boolean): Promise<void> {
  if (!paths.length) return;
  const added = await invoke<[number, string][]>('add_files', { paths });
  await lib.refresh();
  if (addOnly) {
    toast(`已加入媒体库：${added.length} 个文件`);
    return;
  }
  const music = added
    .filter(a => a[1] === 'music')
    .map(a => lib.byId(a[0]))
    .filter(Boolean) as Track[];
  if (music.length) {
    player.playList(music, 0, 'open');
    ui.go('now');
  } else if (added[0]) {
    ui.openItem(added[0][0], added[0][1]);
  } else {
    toast('没有可识别的媒体文件');
  }
}

// ---------------------------------------------------------------- 托盘 / 热键命令

function onCmd(c: string) {
  switch (c) {
    case 'toggle':
      player.toggle();
      break;
    case 'next':
      player.next();
      break;
    case 'prev':
      player.prev();
      break;
    case 'vol_up':
      player.setVol(player.vol + 0.05);
      break;
    case 'vol_down':
      player.setVol(player.vol - 0.05);
      break;
    case 'fav': {
      const t = player.current;
      if (t) lib.toggleFav(t.id).then(v => toast(v ? '已收藏' : '已取消收藏'));
      break;
    }
    case 'lyrics':
      setDesktopLyrics(!lyricsOn).catch(e => toast('桌面歌词：' + String(e)));
      break;
    case 'show':
      if (ui.mini) ui.setMini(false);
      invoke('shell_show_main').catch(() => {});
      break;
    case 'mini':
      ui.setMini(true);
      invoke('shell_show_main').catch(() => {});
      break;
  }
}

/**
 * 外壳初始化（只调用一次）：监听托盘 / 热键命令，处理启动文件，恢复默认音量和桌面歌词，
 * 并挂上防休眠、托盘提示、桌面歌词推送。返回清理函数。
 */
export function initShell(): () => void {
  if (!inTauri || isLyricsWindow()) return () => {};
  const offs: Promise<() => void>[] = [];
  offs.push(listen<string>('shell://cmd', onCmd));
  offs.push(listen<OpenPayload>('shell://open-files', p => openPaths(p.paths, p.addOnly).catch(e => toast(String(e)))));
  offs.push(listen<unknown>('lyrics://ask', () => pushLine(true)));
  lyricsfx.load();

  invoke<OpenPayload | null>('shell_startup_files')
    .then(p => p && openPaths(p.paths, p.addOnly))
    .catch(() => {});
  getSetting<number | null>('default_volume', null).then(v => {
    if (v != null) player.setVol(v / 100);
  });
  getSetting<boolean>('desktop_lyrics', false).then(on => {
    if (on) setDesktopLyrics(true).catch(() => {});
  });

  const stop = $effect.root(() => {
    // 托盘提示显示当前歌曲
    $effect(() => {
      const c = player.current;
      invoke('shell_tray_tooltip', { text: c ? (c.artist ? `${c.title} - ${c.artist}` : c.title) : '极光播放器' }).catch(() => {});
    });
    // 防休眠：视频播放器打开，或沉浸模式且在播放时
    let awake = false;
    $effect(() => {
      const want = ui.videoId != null || (ui.immersive && player.playing);
      if (want === awake) return;
      awake = want;
      invoke('shell_keep_awake', { on: want }).catch(() => {});
    });
    // 桌面歌词：歌词行变化时推给歌词窗口
    $effect(() => {
      if (lyricsOn) pushLine();
    });
  });

  return () => {
    stop();
    offs.forEach(p => p.then(f => f()));
  };
}
