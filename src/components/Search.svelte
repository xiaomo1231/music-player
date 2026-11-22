<script lang="ts">
  import Icon from '../lib/Icon.svelte';
  import Cover from './Cover.svelte';
  import { invoke, inTauri, type Hit } from '../lib/api';
  import { ui } from '../lib/ui.svelte';
  import { lib } from '../lib/library.svelte';
  import { player } from '../lib/player.svelte';

  let q = $state('');
  let hits = $state<Hit[]>([]);
  let open = $state(false);
  let t: ReturnType<typeof setTimeout> | undefined;
  const LABEL: Record<string, string> = { music: '歌曲', anime: '番剧', video: '视频', manga: '漫画', novel: '小说' };
  const groups = $derived(Object.keys(LABEL).map(k => [k, hits.filter(h => h.kind === k)] as const).filter(g => g[1].length));

  function onInput() {
    clearTimeout(t);
    t = setTimeout(async () => {
      if (!q.trim() || !inTauri) { hits = []; return; }
      hits = await invoke<Hit[]>('search', { q });
      open = true;
    }, 140);
  }
  function pick(h: Hit) {
    open = false;
    if (h.kind === 'music') {
      const tr = lib.byId(h.id);
      if (tr) { player.playList([tr], 0, 'search'); ui.go('now'); }
    } else if (h.kind === 'anime' && h.series_id) { ui.seriesId = h.series_id; ui.go('anime'); }
    else if (h.series_id) { ui.seriesId = h.series_id; ui.go('video'); }
    else ui.openItem(h.id, h.kind);
  }
  function key(e: KeyboardEvent) {
    if (e.key === 'Enter' && hits[0]) pick(hits[0]);
    if (e.key === 'Escape') { open = false; (e.target as HTMLInputElement).blur(); e.stopPropagation(); }
  }
</script>
<label class="search"><Icon name="search" />
  <input id="gs" bind:value={q} oninput={onInput} onkeydown={key} onfocus={() => (open = hits.length > 0)} onblur={() => setTimeout(() => (open = false), 150)}
    placeholder="搜索歌曲、艺人、番剧、漫画、小说…（支持拼音首字母）" autocomplete="off" /><kbd>/</kbd></label>
<div class="results" class:open={open && q.trim().length > 0}>
  {#if !hits.length}<h6>没有找到“{q}”</h6>{/if}
  {#each groups as [k, list]}
    <h6>{LABEL[k]} · {list.length}</h6>
    {#each list.slice(0, 8) as h (h.id)}
      <div role="button" tabindex="-1" onmousedown={() => pick(h)}><Cover tag="i" cls="" cover={h.cover} name={h.title} />{h.title}<small>{h.sub}</small></div>
    {/each}
  {/each}
</div>
