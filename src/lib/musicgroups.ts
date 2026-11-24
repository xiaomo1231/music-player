// 音乐库分类浏览：按艺人、专辑、流派、文件夹、格式、年份把歌曲分成一组一组
import type { Track } from './api';

export type Dim = 'artist' | 'album' | 'genre' | 'folder' | 'format' | 'year';
export type GroupSort = 'name' | 'count' | 'plays' | 'added';

export const DIMS: [Dim, string, string][] = [
  ['artist', '艺人', '位艺人'],
  ['album', '专辑', '张专辑'],
  ['genre', '流派', '种流派'],
  ['folder', '文件夹', '个文件夹'],
  ['format', '格式', '种格式'],
  ['year', '年份', '个年份'],
];
export const dimLabel = (d: Dim) => DIMS.find(x => x[0] === d)![1];

export interface Group {
  /** 唯一键（同一维度内） */
  key: string;
  /** 显示名 */
  name: string;
  /** 卡片副标题（专辑：艺人 · 年份） */
  sub: string;
  tracks: Track[];
  /** 封面：组里第一首有封面的歌 */
  cover: string | null;
  hue: number | null;
  plays: number;
  added: number;
  /** 艺人页：这位艺人的专辑数 */
  albums: number;
  /** 未知艺人 / 未知专辑等，排在最后 */
  unknown: boolean;
}

export const UNKNOWN_ARTIST = '未知艺人';
export const UNKNOWN_ALBUM = '未知专辑';
export const VARIOUS = '群星';

/** 合作曲拆成多位艺人：“A / B”“A、B”“A; B”“A feat. B”“A ft. B”“A × B”。“&”“and”不拆（乐队名里常见） */
const SPLIT = /\s*(?:[/／;；、，|×]|\s+(?:feat\.?|ft\.?|featuring)\s+|\s*[（(](?:feat\.?|ft\.?|featuring)\s+)\s*/i;
/** 名字里本来就带分隔符的艺人，不拆 */
const KEEP = ['AC/DC', 'Earth, Wind & Fire', 'Tyler, The Creator'];
export function splitArtists(s: string): string[] {
  const out: string[] = [];
  const kept = KEEP.find(k => s.toLowerCase() === k.toLowerCase());
  if (kept) return [s.trim()];
  // 英文逗号：“周杰伦,费玉清”要拆；“Earth, Wind & Fire”“Tyler, The Creator”这种乐队名不拆
  const parts = s.split(SPLIT).flatMap(p => (p.includes('&') || /,\s*the\s/i.test(p) ? [p] : p.split(/\s*,\s*/)));
  for (const p of parts) {
    const v = p.replace(/[)）]\s*$/, '').trim();
    if (v && !out.some(x => x.toLowerCase() === v.toLowerCase())) out.push(v);
  }
  return out;
}

export const dirOf = (p: string) => p.replace(/[\\/][^\\/]*$/, '');
const baseOf = (p: string) => p.replace(/^.*[\\/]/, '') || p;

/** 专辑的显示艺人：专辑艺人 → 全部歌曲同一艺人 → 群星 */
export function albumArtist(ts: Track[]): string {
  const aa = ts.find(t => t.album_artist)?.album_artist;
  if (aa) return aa;
  const as = new Set(ts.map(t => t.artist).filter(Boolean));
  if (as.size === 1) return [...as][0];
  return as.size ? VARIOUS : UNKNOWN_ARTIST;
}

/** 同名专辑：有专辑艺人按专辑艺人合并，没有就按所在文件夹合并（合辑里每首歌艺人不同） */
export const albumKey = (t: Track) => (t.album || '').trim().toLowerCase() + '\0' + (t.album_artist ? t.album_artist.trim().toLowerCase() : dirOf(t.path).toLowerCase());

/** 专辑内的曲目顺序：碟号 → 音轨号 → 文件名 */
export function albumOrder(a: Track, b: Track): number {
  return (a.disc_no ?? 1) - (b.disc_no ?? 1) || (a.track_no ?? 9999) - (b.track_no ?? 9999) || baseOf(a.path).localeCompare(baseOf(b.path), 'zh', { numeric: true });
}

function keysOf(t: Track, d: Dim): [string, string][] {
  switch (d) {
    case 'artist': {
      const as = splitArtists(t.artist || t.album_artist || '');
      return as.length ? as.map(a => [a.toLowerCase(), a]) : [['', UNKNOWN_ARTIST]];
    }
    case 'album':
      return t.album ? [[albumKey(t), t.album]] : [['', UNKNOWN_ALBUM]];
    case 'genre': {
      const gs = (t.genre || '').split(/\s*[/／;；、,，|]\s*/).map(s => s.trim()).filter(Boolean);
      return gs.length ? gs.map(g => [g.toLowerCase(), g]) : [['', '未知流派']];
    }
    case 'folder': {
      const dir = dirOf(t.path);
      return [[dir.toLowerCase(), dir]];
    }
    case 'format':
      return t.format ? [[t.format.toLowerCase(), t.format]] : [['', '未知格式']];
    case 'year':
      return t.year ? [[String(t.year), String(t.year)]] : [['', '未知年份']];
  }
}

/** 把歌曲按某个维度分组 */
export function groupTracks(tracks: Track[], d: Dim): Group[] {
  const m = new Map<string, { name: string; tracks: Track[] }>();
  for (const t of tracks) {
    for (const [k, name] of keysOf(t, d)) {
      let g = m.get(k);
      if (!g) m.set(k, (g = { name, tracks: [] }));
      g.tracks.push(t);
    }
  }
  const out: Group[] = [];
  for (const [key, g] of m) {
    const ts = d === 'album' ? [...g.tracks].sort(albumOrder) : g.tracks;
    const unknown = key === '';
    let sub = '';
    let albums = 0;
    if (d === 'album') {
      const y = ts.find(t => t.year)?.year;
      sub = unknown ? '' : albumArtist(ts) + (y ? ` · ${y}` : '');
    } else if (d === 'artist') {
      albums = new Set(ts.filter(t => t.album).map(albumKey)).size;
    } else if (d === 'folder') {
      sub = key;
    }
    out.push({
      key,
      name: d === 'folder' ? baseOf(g.name) : g.name,
      sub,
      tracks: ts,
      cover: coverOf(ts),
      hue: ts[0]?.hue ?? null,
      plays: ts.reduce((n, t) => n + t.play_count, 0),
      added: ts.reduce((n, t) => Math.max(n, t.added_at), 0),
      albums,
      unknown,
    });
  }
  return out;
}

/** 组的封面：听得最多的那张专辑里第一张有封面的图（艺人头像也用这个） */
export function coverOf(ts: Track[]): string | null {
  const withCover = ts.filter(t => t.cover);
  if (!withCover.length) return null;
  const score = new Map<string, number>();
  for (const t of withCover) score.set(albumKey(t), (score.get(albumKey(t)) ?? 0) + 1 + t.play_count);
  let best = withCover[0];
  for (const t of withCover) if ((score.get(albumKey(t)) ?? 0) > (score.get(albumKey(best)) ?? 0)) best = t;
  return best.cover;
}

export function sortGroups(gs: Group[], s: GroupSort, d: Dim): Group[] {
  const by: Record<GroupSort, (a: Group, b: Group) => number> = {
    name: (a, b) => (d === 'year' ? b.name.localeCompare(a.name) : a.name.localeCompare(b.name, 'zh', { numeric: true })),
    count: (a, b) => b.tracks.length - a.tracks.length,
    plays: (a, b) => b.plays - a.plays,
    added: (a, b) => b.added - a.added,
  };
  return [...gs].sort((a, b) => +a.unknown - +b.unknown || by[s](a, b) || by.name(a, b));
}

/** 卡片下面那行小字 */
export function groupSub(g: Group, d: Dim): string {
  const n = `${g.tracks.length} 首`;
  if (d === 'artist') return g.albums ? `${g.albums} 张专辑 · ${n}` : n;
  if (d === 'album') return g.sub ? `${g.sub} · ${n}` : n;
  return n;
}

// 记住上次停在哪个视图
const VIEW_KEY = 'mp.music.view';
const SORT_KEY = 'mp.music.groupSort';
export function loadView(): 'songs' | Dim {
  try {
    const v = localStorage.getItem(VIEW_KEY);
    if (v && (v === 'songs' || DIMS.some(d => d[0] === v))) return v as 'songs' | Dim;
  } catch { /* 读不到就用默认 */ }
  return 'songs';
}
export function saveView(v: string) {
  try { localStorage.setItem(VIEW_KEY, v); } catch { /* 忽略 */ }
}
export function loadGroupSort(): GroupSort {
  try {
    const v = localStorage.getItem(SORT_KEY);
    if (v === 'name' || v === 'count' || v === 'plays' || v === 'added') return v;
  } catch { /* 忽略 */ }
  return 'name';
}
export function saveGroupSort(v: GroupSort) {
  try { localStorage.setItem(SORT_KEY, v); } catch { /* 忽略 */ }
}
