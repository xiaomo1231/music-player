// 媒体库数据（从 Rust 读，媒体库变化时自动刷新）
import { invoke, listen, inTauri, type Track, type Playlist, type Stats } from './api';

class Library {
  tracks = $state<Track[]>([]);
  playlists = $state<Playlist[]>([]);
  stats = $state<Stats>({ music: 0, video: 0, anime: 0, manga: 0, novel: 0 });
  /** 每次媒体库变化 +1，各页面据此重新拉数据 */
  version = $state(0);
  loaded = $state(false);

  byId(id: number) { return this.tracks.find(t => t.id === id); }

  async refresh() {
    if (!inTauri) { this.loaded = true; return; }
    const [t, p, s] = await Promise.all([
      invoke<Track[]>('list_tracks'),
      invoke<Playlist[]>('list_playlists'),
      invoke<Stats>('library_stats'),
    ]);
    this.tracks = t;
    this.playlists = p;
    this.stats = s;
    this.version++;
    this.loaded = true;
  }

  async refreshPlaylists() { if (inTauri) this.playlists = await invoke<Playlist[]>('list_playlists'); }

  async toggleFav(id: number) {
    const v = await invoke<boolean>('toggle_favorite', { id });
    const t = this.tracks.find(x => x.id === id);
    if (t) t.favorite = v;
    return v;
  }
}
export const lib = new Library();

let timer: ReturnType<typeof setTimeout> | undefined;
listen('library://changed', () => { clearTimeout(timer); timer = setTimeout(() => lib.refresh(), 400); });
