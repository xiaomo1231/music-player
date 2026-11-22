// 智能列表：内置几种 + 用户自定义规则（保存在设置里）
import type { Track } from './api';
import { invoke, inTauri } from './api';

export interface Rule {
  field: 'plays' | 'skips' | 'lossless' | 'fav' | 'days_since_play' | 'days_since_add' | 'format' | 'artist' | 'genre' | 'duration';
  op: '>' | '<' | '=' | 'has';
  value: string;
}
export interface SmartDef {
  key: string;
  name: string;
  desc: string;
  hue: number;
  pick: (t: Track[]) => Track[];
  custom?: boolean;
  rules?: Rule[];
  limit?: number;
  cover?: string | null;
}

const day = 86400;
const now = () => Date.now() / 1000;
const by = <T>(f: (a: T) => number) => (a: T, b: T) => f(b) - f(a);

export const smartLists: SmartDef[] = [
  { key: 'hot', name: '常听', desc: '播放次数最多', hue: 330, pick: t => t.filter(x => x.play_count >= 3).sort(by(x => x.play_count)).slice(0, 200) },
  { key: 'new', name: '最近添加', desc: '近 30 天入库', hue: 200, pick: t => t.filter(x => now() - x.added_at < 30 * day).sort(by(x => x.added_at)) },
  { key: 'recent', name: '最近播放', desc: '最近听过的', hue: 260, pick: t => t.filter(x => x.last_played).sort(by(x => x.last_played ?? 0)).slice(0, 200) },
  { key: 'fav', name: '我的收藏', desc: '标记为喜欢', hue: 350, pick: t => t.filter(x => x.favorite) },
  { key: 'hifi', name: '无损音质', desc: 'FLAC / APE / WAV / DSD 等', hue: 160, pick: t => t.filter(x => x.lossless) },
  { key: 'cold', name: '很久没听', desc: '超过三个月没播放', hue: 40, pick: t => t.filter(x => x.play_count >= 2 && x.last_played && now() - x.last_played > 90 * day) },
  { key: 'never', name: '从未播放', desc: '库里吃灰的歌', hue: 100, pick: t => t.filter(x => x.play_count === 0) },
];

export const FIELDS: [Rule['field'], string, 'num' | 'bool' | 'text'][] = [
  ['plays', '播放次数', 'num'],
  ['skips', '跳过次数', 'num'],
  ['days_since_play', '距上次播放（天）', 'num'],
  ['days_since_add', '入库天数', 'num'],
  ['duration', '时长（分钟）', 'num'],
  ['lossless', '无损', 'bool'],
  ['fav', '已收藏', 'bool'],
  ['format', '格式包含', 'text'],
  ['artist', '艺人包含', 'text'],
  ['genre', '流派包含', 'text'],
];

export function matchRules(t: Track, rules: Rule[]): boolean {
  return rules.every(r => {
    const v = r.value.trim();
    const num = (x: number) => (r.op === '>' ? x > +v : r.op === '<' ? x < +v : Math.round(x) === +v);
    switch (r.field) {
      case 'plays': return num(t.play_count);
      case 'skips': return num(t.skip_count);
      case 'duration': return num(t.duration / 60);
      case 'lossless': return t.lossless === (v !== 'false');
      case 'fav': return t.favorite === (v !== 'false');
      case 'days_since_play': return num(t.last_played ? (now() - t.last_played) / day : 99999);
      case 'days_since_add': return num((now() - t.added_at) / day);
      case 'format': return t.format.toLowerCase().includes(v.toLowerCase());
      case 'artist': return (t.artist + ' ' + t.album_artist).toLowerCase().includes(v.toLowerCase());
      case 'genre': return t.genre.toLowerCase().includes(v.toLowerCase());
    }
  });
}

export interface StoredList {
  key: string;
  name: string;
  rules: Rule[];
  limit?: number;
  cover?: string | null;
}

class CustomLists {
  list = $state<SmartDef[]>([]);
  /** 被隐藏的内置智能列表 */
  hidden = $state<string[]>([]);
  private raw: StoredList[] = [];
  async load() {
    if (!inTauri) return;
    this.raw = ((await invoke<StoredList[] | null>('get_setting', { key: 'smart_lists' })) ?? []) as StoredList[];
    this.hidden = ((await invoke<string[] | null>('get_setting', { key: 'smart_hidden' })) ?? []) as string[];
    this.build();
  }
  async setHidden(key: string, hide: boolean) {
    this.hidden = hide ? [...new Set([...this.hidden, key])] : this.hidden.filter(k => k !== key);
    await invoke('set_setting', { key: 'smart_hidden', value: this.hidden });
  }
  get(key: string) {
    return this.raw.find(x => x.key === key);
  }
  private build() {
    this.list = this.raw.map((s, i) => ({
      key: s.key,
      name: s.name,
      desc: '自定义规则',
      hue: (i * 67 + 20) % 360,
      custom: true,
      rules: s.rules,
      limit: s.limit,
      cover: s.cover,
      pick: (t: Track[]) => {
        const r = t.filter(x => matchRules(x, s.rules));
        return s.limit ? r.slice(0, s.limit) : r;
      },
    }));
  }
  async save(s: StoredList) {
    const i = this.raw.findIndex(x => x.key === s.key);
    if (i >= 0) this.raw[i] = s;
    else this.raw.push(s);
    await invoke('set_setting', { key: 'smart_lists', value: this.raw });
    this.build();
  }
  async remove(key: string) {
    this.raw = this.raw.filter(x => x.key !== key);
    await invoke('set_setting', { key: 'smart_lists', value: this.raw });
    this.build();
  }
}
export const customLists = new CustomLists();
customLists.load().catch(() => {});
