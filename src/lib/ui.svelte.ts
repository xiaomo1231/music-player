// 界面状态：当前页面、弹窗、提示、沉浸 / 迷你模式、OLED 防烧屏
export type View = 'now' | 'music' | 'video' | 'movie' | 'anime' | 'manga' | 'novel' | 'daily' | 'convert' | 'aisub' | 'get' | 'org' | 'settings';
export type Modal = null | 'scan' | 'eq' | 'oled' | 'ai' | 'keys' | 'playlist-add' | 'about';

const load = <T>(k: string, d: T): T => {
  try { const v = localStorage.getItem('mp.' + k); return v == null ? d : { ...d, ...JSON.parse(v) }; } catch { return d; }
};

export interface Oled { black: boolean; shift: boolean; shiftSec: number; drift: boolean; dim: boolean; dimSec: number; dimLv: number; hide: boolean; stat: boolean }

class UI {
  view = $state<View>('now');
  modal = $state<Modal>(null);
  modalArg = $state<unknown>(null);
  toastMsg = $state('');
  toastShow = $state(false);
  immersive = $state(false);
  mini = $state(false);
  queueOpen = $state(false);
  idle = $state(false);
  dimmed = $state(false);
  refreshRun = $state(0);
  oled = $state<Oled>(load('oled', { black: false, shift: true, shiftSec: 60, drift: true, dim: true, dimSec: 300, dimLv: 0.65, hide: true, stat: false }));
  /** 当前打开的覆盖层：视频播放器 / 阅读器 */
  videoId = $state<number | null>(null);
  videoList = $state<number[]>([]);
  bookId = $state<number | null>(null);
  comicId = $state<number | null>(null);
  seriesId = $state<number | null>(null);
  playlistId = $state<number | null>(null);
  /** 从别处带到转换页的文件 */
  convertPaths = $state<string[]>([]);
  /** 点了关闭按钮 / 托盘退出，等用户选择（见 CloseModal） */
  closeAsk = $state<{ ask: boolean; downloads: number } | null>(null);
  /** 从别处带到 AI 字幕页的视频 */
  subVideo = $state<number | null>(null);
  /** 从顶部搜索框带到自动获取页的关键词（“查看全部”），搜索页读到后自动搜一次并清空 */
  fetchQuery = $state<{ q: string; mk: string } | null>(null);

  setImmersive: (v?: boolean) => void = () => {};
  setMini: (v?: boolean) => void = () => {};

  go(v: View) { this.view = v; }
  /** 按类型打开一个媒体项（视频 / 小说 / 漫画） */
  openItem(id: number, kind: string, list?: number[]) {
    if (kind === 'video' || kind === 'anime') { this.videoList = list ?? [id]; this.videoId = id; }
    else if (kind === 'novel') this.bookId = id;
    else if (kind === 'manga') this.comicId = id;
  }
  open(m: Modal, arg: unknown = null) { this.modal = m; this.modalArg = arg; }
  close() { this.modal = null; }
  saveOled() { try { localStorage.setItem('mp.oled', JSON.stringify(this.oled)); } catch {} }
}
export const ui = new UI();

let tt: ReturnType<typeof setTimeout> | undefined;
export function toast(msg: string) {
  ui.toastMsg = msg;
  ui.toastShow = true;
  clearTimeout(tt);
  tt = setTimeout(() => (ui.toastShow = false), 2400);
}
