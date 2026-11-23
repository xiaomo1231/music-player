<script lang="ts">
  // 新建下载：粘贴磁力链接、种子地址、HTTP 直链（一行一个），或选本地种子文件
  import { inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { fetchApi, MEDIA_TEXT, taskTitleOf } from '../../lib/fetch';

  // 只有这三类下完会自动整理入库
  const KINDS = ['video', 'anime', 'music'];

  let { onqueued }: { onqueued?: () => void } = $props();

  let text = $state('');
  let mk = $state('video');
  let busy = $state(false);

  const lines = $derived(
    text
      .split(/\r?\n/)
      .map(s => s.trim())
      .filter(Boolean),
  );

  async function start(urls: string[]) {
    if (urls.length === 0) return toast('先粘贴磁力链接、种子地址或下载直链');
    if (!inTauri) return toast('请在桌面程序里下载');
    if (busy) return;
    busy = true;
    try {
      const ids = await fetchApi.download(urls.map(url => ({ title: taskTitleOf(url), url, kind: '', mediaKind: mk, sourceId: null, size: null })));
      toast(`已加入下载队列：${ids.length} 个`);
      text = '';
      onqueued?.();
    } catch (err) {
      toast(String(err));
    } finally {
      busy = false;
    }
  }

  async function pickTorrent() {
    if (!inTauri) return toast('请在桌面程序里选择种子文件');
    const { open } = await import('@tauri-apps/plugin-dialog');
    const p = await open({ multiple: true, filters: [{ name: '种子文件', extensions: ['torrent'] }] });
    const files = Array.isArray(p) ? p : typeof p === 'string' ? [p] : [];
    if (files.length) await start(files);
  }
</script>

<div class="nt">
  <textarea class="tin" rows="2" placeholder="粘贴磁力链接（magnet:）、种子地址（.torrent）或下载直链，一行一个" bind:value={text}
    onkeydown={e => e.key === 'Enter' && (e.ctrlKey || e.metaKey) && start(lines)}></textarea>
  <div class="frow">
    <span>下完归到</span>
    <select class="sel" bind:value={mk} aria-label="媒体类型">
      {#each KINDS as k (k)}
        <option value={k}>{MEDIA_TEXT[k]}</option>
      {/each}
    </select>
    <span class="spacer"></span>
    <button class="mp-btn ghost" onclick={pickTorrent} disabled={busy}>选择种子文件…</button>
    <button class="mp-btn" onclick={() => start(lines)} disabled={busy || lines.length === 0}>开始下载{lines.length > 1 ? `（${lines.length}）` : ''}</button>
  </div>
</div>

<style>
  .nt {
    display: grid;
    gap: 8px;
    margin-bottom: 12px;
  }
  .nt textarea {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    font-family: inherit;
    min-height: 52px;
  }
  .frow {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
    font-size: 12.5px;
    color: var(--mp-sub);
  }
  .spacer {
    flex: 1;
  }
</style>
