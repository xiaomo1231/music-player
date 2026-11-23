<script lang="ts">
  // 更换封面：从联网搜到的官方海报里选一张；也可以手动搜、用本地图片、关掉联网封面、恢复自动
  import Modal from '../Modal.svelte';
  import { invoke, inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { changeCover } from '../../lib/items.svelte';
  import type { ArtInfo, ArtKind } from '../../lib/media.svelte';

  interface Candidate {
    source: string; remote_id: string; title: string; names: string[]; year: number | null; format: string;
    episodes: number | null; image: string; summary: string; score: number; name_score: number;
  }
  interface Picker { info: ArtInfo | null; candidates: Candidate[]; query: string; source: string }

  let { kind, id, title, onclose }: { kind: ArtKind; id: number; title: string; onclose: () => void } = $props();

  const SOURCES = [
    { id: 'bangumi', name: 'Bangumi' },
    { id: 'anilist', name: 'AniList' },
    { id: 'tmdb', name: 'TMDB' },
  ];
  const SOURCE_NAME: Record<string, string> = { bangumi: 'Bangumi', anilist: 'AniList', tmdb: 'TMDB' };
  const STATE: Record<string, string> = {
    auto: '自动匹配',
    locked: '手动选定',
    pending: '拿不准，等你确认',
    none: '没有搜到',
    off: '已关闭联网封面',
  };

  let info = $state<ArtInfo | null>(null);
  let list = $state<Candidate[]>([]);
  let query = $state('');
  let source = $state('bangumi');
  let busy = $state(false);
  let choosing = $state('');
  let error = $state('');

  $effect(() => {
    if (!inTauri) return;
    invoke<Picker>('art_picker', { kind, id })
      .then(p => {
        info = p.info;
        list = p.candidates;
        query = p.query;
        source = p.source;
      })
      .catch(e => (error = String(e)));
  });

  async function search() {
    busy = true;
    error = '';
    try {
      list = await invoke<Candidate[]>('art_search', { kind, id, query, source });
      if (!list.length) error = '没有搜到，换个名字（原名、英文名）或换个来源试试';
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  async function choose(c: Candidate) {
    choosing = c.source + c.remote_id;
    try {
      await invoke('art_choose', { kind, id, candidate: c });
      toast('已更换封面');
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      choosing = '';
    }
  }

  async function off() {
    await invoke('art_off', { kind, id }).catch(e => toast(String(e)));
    toast('这部不再使用联网海报');
    onclose();
  }

  async function reset() {
    busy = true;
    try {
      const r = await invoke<ArtInfo | null>('art_reset', { kind, id });
      toast(r?.state === 'auto' ? '已恢复自动匹配' : '已恢复自动；这次没有自动匹配上，可以在这里手动选');
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  const isCurrent = (c: Candidate) => info?.source === c.source && info?.remote_id === c.remote_id && (info?.state === 'auto' || info?.state === 'locked');
  const meta = (c: Candidate) => [SOURCE_NAME[c.source] ?? c.source, c.year, c.format, c.episodes ? `${c.episodes} 集` : ''].filter(Boolean).join(' · ');
</script>

<Modal title="更换封面：{title}" {onclose} wide>
  <p class="note">
    {#if info}当前：{STATE[info.state] ?? info.state}{info.title ? `（${info.title}${info.year ? `，${info.year}` : ''}）` : ''}。{/if}点一张海报就会换上，并锁定不再自动更改。
  </p>
  <div class="cp-bar">
    <input class="tin" bind:value={query} placeholder="作品名，原名或英文名也可以" onkeydown={e => e.key === 'Enter' && search()} />
    <div class="seg">
      {#each SOURCES as s (s.id)}
        <button class:on={source === s.id} onclick={() => (source = s.id)}>{s.name}</button>
      {/each}
    </div>
    <button class="mp-btn" disabled={busy || !query.trim()} onclick={search}>{busy ? '搜索中…' : '搜索'}</button>
  </div>
  {#if error}<p class="note cp-err">{error}</p>{/if}
  <div class="cp-grid">
    {#each list as c (c.source + c.remote_id)}
      <button class="cp-item" class:cur={isCurrent(c)} disabled={!!choosing || !c.image} onclick={() => choose(c)} title={c.summary || c.title}>
        <span class="cp-img" style:background-image={c.image ? `url("${c.image}")` : null}>
          {#if !c.image}<small>没有海报</small>{/if}
          {#if choosing === c.source + c.remote_id}<small>下载中…</small>{/if}
          {#if isCurrent(c)}<i>当前</i>{/if}
        </span>
        <b>{c.title}</b>
        <small>{meta(c)}</small>
      </button>
    {/each}
  </div>
  <div class="rowbtns">
    {#if kind !== 'comic'}
      <button class="mp-btn ghost" onclick={() => { onclose(); changeCover(kind, id); }}>用本地图片…</button>
    {/if}
    <button class="mp-btn ghost" disabled={busy} onclick={reset}>恢复自动匹配</button>
    <button class="mp-btn ghost" onclick={off}>不要联网封面</button>
  </div>
</Modal>

<style>
  .cp-bar { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; margin: 10px 0; }
  .cp-bar .tin { flex: 1 1 220px; }
  .cp-err { color: #ff9b9b; }
  .cp-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(118px, 1fr)); gap: 12px; max-height: 52vh; overflow: auto; padding: 2px; }
  .cp-item { all: unset; cursor: pointer; display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  .cp-item:disabled { cursor: default; opacity: .6; }
  .cp-img { position: relative; aspect-ratio: 2 / 3; border-radius: 10px; background: var(--mp-panel2) center / cover no-repeat; border: 2px solid transparent; display: grid; place-items: center; overflow: hidden; }
  .cp-img small { background: rgba(0, 0, 0, .6); color: #fff; padding: 2px 6px; border-radius: 5px; font-size: 11px; }
  .cp-img i { position: absolute; left: 6px; top: 6px; font-style: normal; font-size: 10px; background: var(--mp-accent); color: #000; padding: 1px 6px; border-radius: 5px; }
  .cp-item:hover:not(:disabled) .cp-img, .cp-item.cur .cp-img { border-color: var(--mp-accent); }
  .cp-item b { font-size: 12.5px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .cp-item small { font-size: 11px; color: var(--mp-sub); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
