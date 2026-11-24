// 书架上的追更：漫画 / 小说 / 番剧作品里显示“追更中”，可以手动检查更新
import { inTauri } from './api';
import { fetchApi, type SubRow } from './fetch';
import { toast } from './ui.svelte';

/** 作品名比较时忽略大小写和空格（和后端分组一样） */
export const workKey = (s: string) => s.toLowerCase().replace(/\s+/g, '');

class Follow {
  subs = $state<SubRow[]>([]);
  /** 正在检查的追更 id */
  checking = $state<Record<number, boolean>>({});
  /** 上次手动检查的结果文字 */
  result = $state<Record<number, string>>({});
  #loaded = false;

  async load(force = false) {
    if (!inTauri || (this.#loaded && !force)) return;
    this.#loaded = true;
    try {
      this.subs = await fetchApi.subs();
    } catch {
      /* 旧版本没有追更时忽略 */
    }
  }

  /** 某个类型里同名作品的追更 */
  of(kind: string, name: string): SubRow[] {
    const k = workKey(name);
    return this.subs.filter(s => s.media_kind === kind && workKey(s.query) === k);
  }

  /** 这个类型里书架上还没有对应作品的追更（还没下载过） */
  orphans(kind: string, names: string[]): SubRow[] {
    const have = new Set(names.map(workKey));
    return this.subs.filter(s => s.media_kind === kind && !have.has(workKey(s.query)));
  }

  async check(s: SubRow, unit: string) {
    if (this.checking[s.id]) return;
    this.checking[s.id] = true;
    try {
      const r = await fetchApi.subCheck(s.id);
      const msg = r.titles.length ? `发现 ${r.titles.length} ${unit}新内容，已加入下载` : '没有更新内容';
      this.result[s.id] = msg;
      toast(`${s.query}：${msg}`);
      await this.load(true);
    } catch (err) {
      this.result[s.id] = `检查失败：${err}`;
      toast(String(err));
    } finally {
      this.checking[s.id] = false;
    }
  }

  async cancel(s: SubRow) {
    try {
      await fetchApi.unsubscribe(s.id);
      toast(`已取消追更：${s.query}`);
      await this.load(true);
    } catch (err) {
      toast(String(err));
    }
  }
}

export const follow = new Follow();

export const UNIT: Record<string, string> = { anime: '集', manga: '话', novel: '章' };

/** 上次检查：刚刚 / 5 分钟前 / 3 小时前 / 2 天前 */
export function agoText(sec: number | null): string {
  if (!sec) return '还没检查过';
  const d = Math.max(0, Date.now() / 1000 - sec);
  if (d < 60) return '刚刚检查过';
  if (d < 3600) return `${Math.floor(d / 60)} 分钟前检查过`;
  if (d < 86400) return `${Math.floor(d / 3600)} 小时前检查过`;
  return `${Math.floor(d / 86400)} 天前检查过`;
}
