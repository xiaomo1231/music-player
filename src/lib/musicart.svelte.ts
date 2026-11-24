// 歌曲 / 专辑封面补全（后端 musicart.rs）：更换封面面板、批量重新获取、后台进度
import { invoke, listen, inTauri } from './api';
import { toast } from './ui.svelte';

export interface MusicArtProgress { running: boolean; done: number; total: number; found: number; manual: boolean }

class MusicArt {
  /** 打开中的“更换封面”面板：按这首歌所在的专辑换 */
  picker = $state<{ track: number; title: string } | null>(null);
  progress = $state<MusicArtProgress>({ running: false, done: 0, total: 0, found: 0, manual: false });
}
export const musicArt = new MusicArt();

export function openMusicCover(track: number, title: string) {
  musicArt.picker = { track, title };
}

if (inTauri) {
  listen<MusicArtProgress>('musicart://progress', p => {
    const was = musicArt.progress.running;
    musicArt.progress = p;
    // 后台自动补的不打扰，补上了才提示一句；手动重新获取的结果由 refetchCovers 提示
    if (was && !p.running && !p.manual && p.found) toast(`已自动补上 ${p.found} 张专辑封面`);
  });
  invoke<MusicArtProgress>('music_art_status').then(p => (musicArt.progress = p)).catch(() => {});
}

/** 批量重新获取：不传歌曲时处理所有还没封面的专辑 */
export async function refetchCovers(tracks: number[] = []) {
  if (musicArt.progress.running) return toast('正在获取封面，请稍等');
  toast(tracks.length ? `开始重新获取 ${tracks.length} 首歌所在专辑的封面` : '开始重新获取缺失的封面');
  try {
    const [total, found] = await invoke<[number, number]>('music_art_refetch', { tracks });
    if (!total) toast('没有需要联网获取封面的专辑（已有封面或手动选过的不会重新获取）');
    else if (found) toast(`封面获取完成：${total} 张专辑里补上了 ${found} 张`);
    else toast(`查了 ${total} 张专辑，没有找到能确定的封面，可以右键歌曲“更换封面”手动选`);
  } catch (e) {
    toast(String(e));
  }
}
