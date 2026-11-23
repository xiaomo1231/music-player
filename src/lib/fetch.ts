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
  provider: '下载源',
};

/** 已接入的下载源（软件默认没有，开发说明见 docs/下载源接口.md） */
export interface ProviderInfo {
  id: string;
  name: string;
  media_kinds: string[];
  note: string;
}

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

export const MEDIA_TEXT: Record<string, string> = { anime: '番剧', manga: '漫画', novel: '小说', music: '音乐', video: '视频' };

/** 集数单位：番剧“集”、漫画“话”、小说“章” */
export const EP_UNIT: Record<string, string> = { anime: '集', manga: '话', novel: '章', video: '集' };

/** 第 3 集 / 第 12.5 话 */
export function epText(kind: string, ep: number | null | undefined): string {
  if (ep === null || ep === undefined) return '';
  return `第 ${ep} ${EP_UNIT[kind] ?? '集'}`;
}

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
  providers: () => invoke<ProviderInfo[]>('fetch_providers'),
};

// ---------- 资料源（作品资料：封面、简介、分集；见 docs/漫画番剧来源接口.md） ----------

export interface MetaInfo {
  id: string;
  name: string;
  media_kinds: string[];
  note: string;
  link_template: string;
}

export interface MetaSubject {
  id: string;
  title: string;
  title_cn: string;
  aliases: string[];
  media_kind: string;
  year: number | null;
  date: string;
  cover: string;
  summary: string;
  total: number | null;
  rating: number | null;
  tags: string[];
  staff: string[];
}

export interface MetaCard {
  provider: string;
  provider_name: string;
  link: string;
  subject: MetaSubject;
}

export interface MetaOut {
  cards: MetaCard[];
  notes: string[];
}

export interface MetaEpisode {
  sort: number;
  title: string;
  title_cn: string;
  airdate: string;
  duration: number | null;
  special: boolean;
}

export const metaApi = {
  providers: () => invoke<MetaInfo[]>('meta_providers'),
  search: (query: string, mediaKind: string) => invoke<MetaOut>('meta_search', { query, mediaKind }),
  episodes: (provider: string, subjectId: string) => invoke<MetaEpisode[]>('meta_episodes', { provider, subjectId }),
};

/** 支持这个类型的来源 / 资料源（media_kinds 为空表示都支持） */
export const supportsKind = (kinds: string[], mk: string) => kinds.length === 0 || kinds.includes(mk);

/** 下载源的一个档位 */
export interface ProviderQuality {
  key: string;
  label: string;
  format: string;
  bitrate: number | null;
  size: number | null;
}

/** 下载源搜索结果的完整信息（带在 Hit.url 里，见 docs/下载源接口.md） */
export interface ProviderItem {
  id: string;
  title: string;
  artist: string;
  album: string;
  year: number | null;
  track: number | null;
  duration: number | null;
  cover: string;
  media_kind: string;
  /** 作品名（番剧 / 漫画 / 小说） */
  series: string;
  season: number | null;
  /** 第几集 / 话 / 章 */
  episode: number | null;
  qualities: ProviderQuality[];
}

function providerParams(h: Hit): URLSearchParams | null {
  if (h.kind !== 'provider' || !h.url.startsWith('provider://')) return null;
  const i = h.url.indexOf('?');
  return i < 0 ? null : new URLSearchParams(h.url.slice(i + 1));
}

/** 下载源结果里的曲目信息；不是下载源结果时返回 null */
export function providerItem(h: Hit): ProviderItem | null {
  const d = providerParams(h)?.get('d');
  if (!d) return null;
  try {
    return JSON.parse(d) as ProviderItem;
  } catch {
    return null;
  }
}

/** 当前选中的档位 key（空串表示默认档位） */
export function providerQualityKey(h: Hit): string {
  return providerParams(h)?.get('q') ?? '';
}

/** 换档位：返回改好下载地址的新 Hit（大小也换成该档位的） */
export function withQuality(h: Hit, key: string): Hit {
  const p = providerParams(h);
  if (!p) return h;
  p.set('q', key);
  const q = providerItem(h)?.qualities.find(x => x.key === key);
  return { ...h, url: h.url.slice(0, h.url.indexOf('?') + 1) + p.toString(), size: q?.size ?? h.size };
}

/** 秒数显示成 3:25 */
export function fmtDur(sec: number | null | undefined): string {
  if (!sec || sec <= 0) return '';
  const m = Math.floor(sec / 60);
  const s = Math.round(sec % 60);
  return `${m}:${String(s).padStart(2, '0')}`;
}

/** 手动新建下载时的任务名：磁力链接取 dn，其他取地址最后一段 */
export function taskTitleOf(url: string): string {
  if (url.startsWith('magnet:')) {
    const dn = new URLSearchParams(url.slice(url.indexOf('?') + 1)).get('dn');
    if (dn) return dn;
    const h = /btih:([0-9a-z]+)/i.exec(url)?.[1];
    return h ? `磁力 ${h.slice(0, 8)}` : '磁力链接';
  }
  const last = url.split(/[\\/]/).filter(Boolean).pop() ?? url;
  try {
    return decodeURIComponent(last.split('?')[0]) || url;
  } catch {
    return last.split('?')[0] || url;
  }
}

/** 由地址和类型推断的文案 */
export function hitKindText(kind: string): string {
  return { magnet: '磁力', torrent: '种子', http: '直链', ytdlp: '网站', file: '本地文件', provider: '下载源' }[kind] ?? kind;
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
