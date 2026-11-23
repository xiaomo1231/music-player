<script lang="ts">
  // 新建下载：粘贴磁力链接、种子地址、HTTP 直链（一行一个），或选本地种子文件
  // 磁力 / 种子先弹确认页（列出种子里的文件和体积），勾选后只下载选中的文件
  import { inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { fetchApi, isBtUrl, MEDIA_TEXT, taskTitleOf, type DlItem } from '../../lib/fetch';
  import TorrentQueue from './TorrentQueue.svelte';

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

  /** 等待确认的磁力 / 种子，由 TorrentQueue 一个一个弹确认页 */
  let pending = $state<DlItem[]>([]);
  let directCount = 0;

  async function start(urls: string[]) {
    if (urls.length === 0) return toast('先粘贴磁力链接、种子地址或下载直链');
    if (!inTauri) return toast('请在桌面程序里下载');
    if (busy || pending.length) return;
    busy = true;
    directCount = 0;
    const direct = urls.filter(u => !isBtUrl(u));
    const bt = urls.filter(isBtUrl);
    try {
      if (direct.length) {
        const ids = await fetchApi.download(direct.map(url => ({ title: taskTitleOf(url), url, kind: '', mediaKind: mk, sourceId: null, size: null })));
        directCount = ids.length;
        onqueued?.();
      }
      text = '';
      // 种子标题留空：确认后用种子里的名字
      if (bt.length) pending = bt.map(url => ({ title: '', url, kind: '', mediaKind: mk, sourceId: null, size: null }));
      else if (directCount) toast(`已加入下载队列：${directCount} 个`);
    } catch (err) {
      toast(String(err));
    } finally {
      busy = false;
    }
  }

  function done(added: number) {
    pending = [];
    const n = added + directCount;
    if (n) toast(`已加入下载队列：${n} 个`);
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
    <button class="mp-btn ghost" onclick={pickTorrent} disabled={busy || pending.length > 0}>选择种子文件…</button>
    <button class="mp-btn" onclick={() => start(lines)} disabled={busy || pending.length > 0 || lines.length === 0}>开始下载{lines.length > 1 ? `（${lines.length}）` : ''}</button>
  </div>
</div>

{#if pending.length}
  <TorrentQueue items={pending} ondone={done} {onqueued} />
{/if}

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
