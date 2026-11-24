// 搜索页：下载源结果按“来源 + 作品”折叠成作品卡，章节的勾选、排序、分页等纯函数
import type { Group, Hit } from './fetch';

/** 一张作品卡：一个来源里的一部作品 */
export interface WorkCard {
  key: string;
  work: string;
  sourceId: number;
  sourceName: string;
  /** 这张卡在 out.hits 里的条目（作品条目或已经是各集） */
  indices: number[];
}

/** 一组搜索结果拆成作品卡（下载源）和版本列表（磁力、RSS、网盘等，保持原来的逐条显示） */
export function splitGroup(hits: Hit[], g: Group): { cards: WorkCard[]; rows: number[] } {
  const cards: WorkCard[] = [];
  const rows: number[] = [];
  for (const i of g.indices) {
    const h = hits[i];
    if (h.kind !== 'provider') {
      rows.push(i);
      continue;
    }
    const key = `${h.source_id}|${g.work}`;
    const c = cards.find(x => x.key === key);
    if (c) c.indices.push(i);
    else cards.push({ key, work: g.work, sourceId: h.source_id, sourceName: h.source_name, indices: [i] });
  }
  return { cards, rows };
}

/** 卡里还有没展开的作品条目，点开时要去取章节 */
export const needsEpisodes = (hits: Hit[], c: WorkCard) => c.indices.some(i => hits[i].expand);

/** 这张卡已知的各集（没展开的作品条目不算） */
export function knownEpisodes(hits: Hit[], c: WorkCard): Hit[] {
  return c.indices.map(i => hits[i]).filter(h => !h.expand);
}

/** 按集数排：有集数的在前按集数，没集数的保持原顺序排在后面 */
export function sortEpisodes(list: Hit[], desc = false): Hit[] {
  const withIdx = list.map((h, i) => ({ h, i }));
  withIdx.sort((a, b) => {
    const x = a.h.episode;
    const y = b.h.episode;
    if (x != null && y != null) return x - y || a.i - b.i;
    if (x != null) return -1;
    if (y != null) return 1;
    return a.i - b.i;
  });
  const out = withIdx.map(x => x.h);
  return desc ? out.reverse() : out;
}

/** 去掉同一地址的重复条目（同名的几部作品展开后可能重叠） */
export function dedupe(list: Hit[]): Hit[] {
  const seen = new Set<string>();
  return list.filter(h => !seen.has(h.url) && (seen.add(h.url), true));
}

/** 最新一话的集数 */
export function latestEpisode(list: Hit[]): number | null {
  let m: number | null = null;
  for (const h of list) if (h.episode != null && (m == null || h.episode > m)) m = h.episode;
  return m;
}

/**
 * 区间勾选：有集数时按集数（第 a–b 话），否则按列表里的序号（第 a–b 个，从 1 数）。
 * a、b 写反了也行；返回要选中的地址
 */
export function rangeUrls(list: Hit[], a: number, b: number): string[] {
  const lo = Math.min(a, b);
  const hi = Math.max(a, b);
  const sorted = sortEpisodes(list);
  const byEp = sorted.some(h => h.episode != null);
  return sorted.filter((h, i) => (byEp ? h.episode != null && h.episode >= lo && h.episode <= hi : i + 1 >= lo && i + 1 <= hi)).map(h => h.url);
}

/** 一次显示多少话，往下点“再显示”加载下一批 */
export const PAGE = 60;
