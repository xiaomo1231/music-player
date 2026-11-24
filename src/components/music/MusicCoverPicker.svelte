<script lang="ts">
  // 歌曲的“更换封面”：按这首歌所在的专辑换（整张专辑一起），可以联网搜、用本地图片、只换这一首、恢复自动
  import Modal from '../Modal.svelte';
  import { invoke, inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { pickImage, changeCover } from '../../lib/items.svelte';

  interface Cand {
    source: string; remote_id: string; title: string; artist: string; artists: string[]; song: string;
    year: number | null; tracks: number | null; thumb: string; image: string; score: number; title_score: number; artist_score: number;
  }
  interface Picker {
    key: string; artist: string; album: string; song: string; by_song: boolean; count: number;
    state: string | null; current: string | null; candidates: Cand[]; query: string; online: boolean;
  }

  let { track, title, onclose }: { track: number; title: string; onclose: () => void } = $props();

  const SOURCES = [
    { id: 'netease', name: '网易云' },
    { id: 'itunes', name: 'iTunes' },
    { id: 'musicbrainz', name: 'MusicBrainz' },
  ];
  const SOURCE_NAME: Record<string, string> = { netease: '网易云', itunes: 'iTunes', musicbrainz: 'MusicBrainz', local: '本地图片' };
  const STATE: Record<string, string> = {
    auto: '自动匹配',
    locked: '手动选定',
    pending: '找到了几个，但拿不准，等你挑',
    none: '没有找到',
  };

  let info = $state<Picker | null>(null);
  let list = $state<Cand[]>([]);
  let query = $state('');
  let source = $state('netease');
  let bySong = $state(false);
  let busy = $state(false);
  let choosing = $state('');
  let error = $state('');

  $effect(() => {
    if (!inTauri) return;
    invoke<Picker>('music_art_picker', { track })
      .then(p => {
        info = p;
        list = p.candidates;
        query = p.query;
        bySong = p.by_song;
        // 还没查过：打开就先搜一次
        if (!p.candidates.length && p.query) search();
      })
      .catch(e => (error = String(e)));
  });

  async function search() {
    if (!query.trim()) return;
    busy = true;
    error = '';
    try {
      list = await invoke<Cand[]>('music_art_search', { track, query, source, song: bySong });
      if (!list.length) error = '没有搜到，换个写法（英文名、繁体、只写专辑名）或换个来源试试';
    } catch (e) {
      error = `搜索失败：${e}`;
    } finally {
      busy = false;
    }
  }

  const scope = $derived(info && !info.by_song && info.count > 1 ? `整张专辑（${info.count} 首）` : '这首歌');

  async function choose(c: Cand) {
    choosing = c.source + c.remote_id;
    try {
      await invoke('music_art_choose', { track, candidate: c });
      toast(`已更换${scope}的封面`);
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      choosing = '';
    }
  }

  async function local() {
    const p = await pickImage();
    if (!p) return;
    try {
      await invoke('music_art_local', { track, image: p });
      toast(`已更换${scope}的封面`);
      onclose();
    } catch (e) {
      error = String(e);
    }
  }

  async function onlyThis() {
    onclose();
    await changeCover('track', track);
  }

  async function reset() {
    busy = true;
    try {
      await invoke('item_set_cover', { kind: 'track', id: track, image: null });
      const [, found] = await invoke<[number, number]>('music_art_reset', { track });
      toast(found ? '已恢复自动，重新找到了封面' : '已恢复自动');
      onclose();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  const isCurrent = (c: Cand) => info?.current === `${c.source}:${c.remote_id}` && (info?.state === 'auto' || info?.state === 'locked');
  const meta = (c: Cand) => [SOURCE_NAME[c.source] ?? c.source, c.year, c.tracks ? `${c.tracks} 首` : ''].filter(Boolean).join(' · ');
</script>

<Modal title="更换封面：{info && !info.by_song ? info.album : title}" {onclose} wide>
  <p class="note">
    {#if info}
      {info.artist || '未知艺人'}{info.by_song ? ` · 单曲「${info.song}」` : ` · 专辑「${info.album}」`}。
      {#if info.state}当前：{STATE[info.state] ?? info.state}。{/if}
      点一张就会换到{scope}上，以后不会自动更改。
    {/if}
  </p>
  {#if info && !info.online}<p class="note cp-warn">“联网补全元数据”没有打开，后台不会自动联网找封面；这里手动搜索不受影响。</p>{/if}
  <div class="cp-bar">
    <input class="tin" bind:value={query} placeholder="艺人 专辑名，或者 歌名 艺人" onkeydown={e => e.key === 'Enter' && search()} />
    <div class="seg">
      {#each SOURCES as s (s.id)}
        <button class:on={source === s.id} onclick={() => (source = s.id)}>{s.name}</button>
      {/each}
    </div>
    <label class="cp-song"><input type="checkbox" bind:checked={bySong} disabled={source === 'musicbrainz'} /> 按歌名搜</label>
    <button class="mp-btn" disabled={busy || !query.trim()} onclick={search}>{busy ? '搜索中…' : '搜索'}</button>
  </div>
  {#if error}<p class="note cp-err">{error}</p>{/if}
  <div class="cp-grid">
    {#each list as c (c.source + c.remote_id)}
      <button class="cp-item" class:cur={isCurrent(c)} disabled={!!choosing || !c.image} onclick={() => choose(c)}
        title={`${c.title}${c.song ? `（歌曲：${c.song}）` : ''}\n${c.artist}`}>
        <span class="cp-img" style:background-image={c.thumb ? `url("${c.thumb}")` : null}>
          {#if choosing === c.source + c.remote_id}<small>下载中…</small>{/if}
          {#if isCurrent(c)}<i>当前</i>{/if}
        </span>
        <b>{c.title}</b>
        <small>{c.artist}</small>
        <small>{meta(c)}</small>
      </button>
    {/each}
  </div>
  <div class="rowbtns">
    <button class="mp-btn ghost" onclick={local}>用本地图片…</button>
    {#if info && !info.by_song && info.count > 1}<button class="mp-btn ghost" onclick={onlyThis}>只换这一首…</button>{/if}
    <button class="mp-btn ghost" disabled={busy} onclick={reset}>恢复自动</button>
  </div>
</Modal>

<style>
  .cp-bar { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; margin: 10px 0; }
  .cp-bar .tin { flex: 1 1 220px; }
  .cp-song { display: inline-flex; gap: 4px; align-items: center; font-size: 12.5px; color: var(--mp-sub); white-space: nowrap; }
  .cp-err { color: #ff9b9b; }
  .cp-warn { color: #ffd27a; }
  .cp-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(132px, 1fr)); gap: 12px; max-height: 52vh; overflow: auto; padding: 2px; }
  .cp-item { all: unset; cursor: pointer; display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .cp-item:disabled { cursor: default; opacity: .6; }
  .cp-img { position: relative; aspect-ratio: 1; border-radius: 10px; background: var(--mp-panel2) center / cover no-repeat; border: 2px solid transparent; display: grid; place-items: center; overflow: hidden; }
  .cp-img small { background: rgba(0, 0, 0, .6); color: #fff; padding: 2px 6px; border-radius: 5px; font-size: 11px; }
  .cp-img i { position: absolute; left: 6px; top: 6px; font-style: normal; font-size: 10px; background: var(--mp-accent); color: #000; padding: 1px 6px; border-radius: 5px; }
  .cp-item:hover:not(:disabled) .cp-img, .cp-item.cur .cp-img { border-color: var(--mp-accent); }
  .cp-item b { font-size: 12.5px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .cp-item small { font-size: 11px; color: var(--mp-sub); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
