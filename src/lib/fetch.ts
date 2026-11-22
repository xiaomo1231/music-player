// 自动获取：与后端 fetch 模块对应的类型、命令封装和文案
import { invoke } from './api';

export interface Source {
  id: number;
  kind: string;
  name: string;
  url: string;
  enabled: boolean;
  created_at: number;
  /** 网页来源：rules 或 ai_direct */
  mode: string;
  has_rule: boolean;
}

export interface Hit {
  source_id: number;
  source_name: string;
  title: string;
  url: string;
  size: number | null;
  date: string | null;
  /** magnet / torrent / http / ytdlp / file */
  kind: string;
  work: string;
  season: number | null;
  episode: number | null;
  group: string | null;
  resolution: string | null;
  special: boolean;
  rule_updated: boolean;
}

export interface Group {
  work: string;
  indices: number[];
}

export interface SearchOut {
  hits: Hit[];
  groups: Group[];
  notes: string[];
}

export interface Pick {
  index: number;
  reason: string;
}

export interface TaskRow {
  id: number;
  source_id: number | null;
  title: string;
  kind: string;
  url: string;
  /** queued / downloading / paused / seeding / organizing / done / manual / failed / canceled */
  status: string;
  progress: number;
  speed: number;
  size: number;
  save_path: string;
  error: string;
  created_at: number;
  finished_at: number | null;
  media_kind: string;
}

export interface FetchSettings {
  download_dir: string;
  organize_dir: string;
  concurrency: number;
  speed_kbps: number;
  seed_policy: 'stop' | 'ratio' | string;
  seed_ratio: number;
  /** B 站清晰度上限（画面高度），0 = 账号能拿到的最高 */
  bili_quality: number;
  bili_danmaku: boolean;
  bili_subs: boolean;
}

export interface SubRow {
  id: number;
  source_id: number;
  source_name: string;
  query: string;
  media_kind: string;
  last_check: number | null;
  seen: number;
}

export interface SourceStep {
  step: string;
  status: 'run' | 'ok' | 'warn' | 'error';
  message: string;
  url: string;
}

export interface DlItem {
  title: string;
  url: string;
  kind: string;
  mediaKind: string;
  sourceId: number | null;
  size: number | null;
}

export const KIND_TEXT: Record<string, string> = {
  rss: '订阅链接',
  ytdlp: '视频网站',
  dir: '网盘 / NAS',
  web: '网页（AI 解析）',
};

export const STATUS_TEXT: Record<string, string> = {
  queued: '排队中',
  downloading: '下载中',
  paused: '已暂停',
  seeding: '做种中',
  organizing: '整理入库中',
  done: '已整理入库',
  manual: '需要手动整理',
  failed: '失败',
  canceled: '已取消',
};

export const MEDIA_TEXT: Record<string, string> = { anime: '番剧', music: '音乐', video: '视频' };

/** 进行中的任务（可以暂停、取消） */
export const isActive = (s: string) => s === 'queued' || s === 'downloading' || s === 'seeding' || s === 'organizing';
/** 可以“继续”的任务 */
export const canResume = (s: string) => s === 'paused' || s === 'failed' || s === 'canceled';

export const fetchApi = {
  sources: () => invoke<Source[]>('fetch_sources'),
  addSource: (kind: string, url: string, name?: string, searchTemplate?: string) =>
    invoke<Source>('fetch_add_source', { kind, url, name, searchTemplate }),
  removeSource: (id: number) => invoke<void>('fetch_remove_source', { id }),
  rebuildRule: (id: number) => invoke<Source>('fetch_rebuild_rule', { id }),
  search: (query: string, mediaKind: string) => invoke<SearchOut>('fetch_search', { query, mediaKind }),
  aiPick: (results: Hit[], prefs: string) => invoke<Pick[]>('fetch_ai_pick', { results, prefs }),
  download: (items: DlItem[]) => invoke<number[]>('fetch_download', { items }),
  tasks: () => invoke<TaskRow[]>('fetch_tasks'),
  pause: (id: number) => invoke<void>('fetch_pause', { id }),
  resume: (id: number) => invoke<void>('fetch_resume', { id }),
  retry: (id: number) => invoke<void>('fetch_retry', { id }),
  cancel: (id: number, deleteFiles: boolean) => invoke<void>('fetch_cancel', { id, deleteFiles }),
  getSettings: () => invoke<FetchSettings>('fetch_get_settings'),
  setSettings: (value: FetchSettings) => invoke<FetchSettings>('fetch_set_settings', { value }),
  subscribe: (sourceId: number, query: string, mediaKind: string) =>
    invoke<number>('fetch_subscribe', { sourceId, query, mediaKind }),
  unsubscribe: (id: number) => invoke<void>('fetch_unsubscribe', { id }),
  subs: () => invoke<SubRow[]>('fetch_subs'),
};

/** 由地址和类型推断的文案 */
export function hitKindText(kind: string): string {
  return { magnet: '磁力', torrent: '种子', http: '直链', ytdlp: '网站', file: '本地文件' }[kind] ?? kind;
}

/** 大小、速度的显示（没有值时给出 —） */
export function fmtBytes(n: number | null | undefined): string {
  if (!n || n <= 0) return '—';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let v = n;
  let i = 0;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(i === 0 ? 0 : v >= 100 ? 0 : 1)} ${u[i]}`;
}

// ---------- B 站 ----------

export interface BiliAccount {
  logged_in: boolean;
  /** 保存过登录凭据但已经失效 */
  expired: boolean;
  name: string;
  vip: boolean;
  vip_label: string;
}

export interface BiliQr {
  key: string;
  url: string;
  svg: string;
}

export interface BiliParsed {
  title: string;
  /** bangumi / video / list */
  kind: string;
  /** anime / video */
  media_kind: string;
  entries: { title: string; url: string }[];
}

export interface YtdlpStatus {
  installed: boolean;
  version: string;
  path: string;
  managed: boolean;
}

export const BILI_QUALITY: { value: number; label: string }[] = [
  { value: 0, label: '最高（4K / HDR 需要大会员）' },
  { value: 2160, label: '4K' },
  { value: 1080, label: '1080P' },
  { value: 720, label: '720P' },
  { value: 480, label: '480P' },
];

export const biliApi = {
  account: () => invoke<BiliAccount>('fetch_bili_account'),
  qrStart: () => invoke<BiliQr>('fetch_bili_qr_start'),
  /** waiting / scanned / expired / ok */
  qrPoll: (key: string) => invoke<string>('fetch_bili_qr_poll', { key }),
  logout: () => invoke<void>('fetch_bili_logout'),
  parse: (url: string) => invoke<BiliParsed>('fetch_bili_parse', { url }),
  ytdlpStatus: () => invoke<YtdlpStatus>('fetch_ytdlp_status'),
  ytdlpInstall: () => invoke<YtdlpStatus>('fetch_ytdlp_install'),
};
