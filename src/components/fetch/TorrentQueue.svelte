<script lang="ts">
  // 一批磁力 / 种子逐个弹确认页（TorrentConfirm），确认后只下载勾选的文件；手动新建下载和搜索结果共用
  import { toast } from '../../lib/ui.svelte';
  import { fetchApi, taskTitleOf, type DlItem, type TorrentInfo } from '../../lib/fetch';
  import TorrentConfirm from './TorrentConfirm.svelte';

  let {
    items,
    ondone,
    onqueued,
  }: {
    /** 要确认的磁力 / 种子；title 为空时用种子里的名字 */
    items: DlItem[];
    /** 全部处理完，参数是加入队列的任务数 */
    ondone: (added: number) => void;
    onqueued?: () => void;
  } = $props();

  let i = $state(0);
  let added = 0;
  const cur = $derived(items[i]);

  async function enqueue(list: DlItem[]) {
    try {
      const ids = await fetchApi.download(list);
      added += ids.length;
      onqueued?.();
    } catch (err) {
      toast(String(err));
    }
  }

  function next() {
    if (i + 1 >= items.length) ondone(added);
    else i++;
  }

  async function confirm(info: TorrentInfo, files: number[] | null, size: number, mediaKind: string) {
    const it = cur;
    await enqueue([{ ...it, title: it.title.trim() || info.name, url: info.torrentPath, kind: 'torrent', mediaKind, size, files }]);
    next();
  }

  /** 读不到种子信息：照旧整个下载 */
  async function fallback(mediaKind: string) {
    const it = cur;
    await enqueue([{ ...it, title: it.title.trim() || taskTitleOf(it.url), mediaKind, files: null }]);
    next();
  }

  /** 剩下的（包括当前这个）都不再确认，整个下载 */
  async function restAll() {
    const rest = items.slice(i).map(it => ({ ...it, title: it.title.trim() || taskTitleOf(it.url), files: null }));
    await enqueue(rest);
    ondone(added);
  }
</script>

{#if cur}
  {#key i}
    <TorrentConfirm
      url={cur.url}
      mediaKind={cur.mediaKind}
      step={items.length > 1 ? `${i + 1} / ${items.length}` : ''}
      rest={items.length - i}
      onconfirm={confirm}
      onfallback={fallback}
      onskip={next}
      onrest={items.length - i > 1 ? restAll : undefined}
    />
  {/key}
{/if}
