<script lang="ts">
  // 文件信息
  import Modal from './Modal.svelte';
  import { ui } from '../lib/ui.svelte';
  import { invoke, type Track } from '../lib/api';
  import { fmtT, fmtSize, fmtDate } from '../lib/util';
  const t = ui.modalArg as Track;
  const rows: [string, string][] = [
    ['标题', t.title],
    ['艺人', t.artist || '—'],
    ['专辑', t.album || '—'],
    ['专辑艺人', t.album_artist || '—'],
    ['音轨', t.track_no ? String(t.track_no) + (t.disc_no ? ` / 碟 ${t.disc_no}` : '') : '—'],
    ['年份', t.year ? String(t.year) : '—'],
    ['流派', t.genre || '—'],
    ['时长', fmtT(t.duration)],
    ['格式', `${t.format}${t.lossless ? '（无损）' : ''}`],
    ['码率', t.bitrate ? `${t.bitrate} kbps` : '—'],
    ['采样', t.sample_rate ? `${(t.sample_rate / 1000).toFixed(1)} kHz${t.bit_depth ? ` / ${t.bit_depth} bit` : ''}` : '—'],
    ['大小', fmtSize(t.size)],
    ['播放次数', `${t.play_count} 次（跳过 ${t.skip_count} 次）`],
    ['最后播放', fmtDate(t.last_played) || '从未'],
    ['加入时间', fmtDate(t.added_at)],
  ];
</script>

<Modal title="文件信息" onclose={() => ui.close()}>
  <div class="kv selectable" style="margin-top:8px">
    {#each rows as [k, v]}<span>{k}</span><span>{v}</span>{/each}
    <span>路径</span><span style="word-break:break-all;font-family:Consolas,monospace;font-size:12px">{t.path}</span>
  </div>
  <div class="rowbtns"><button class="mp-btn ghost" onclick={() => invoke('show_in_folder', { path: t.path })}>打开所在文件夹</button></div>
</Modal>
