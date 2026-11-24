<script lang="ts">
  import Icon from '../../lib/Icon.svelte';
  import Cover from '../Cover.svelte';
  import MoreButton from '../MoreButton.svelte';
  import { openMenu, mediaEntries, type MenuEntry } from '../../lib/items.svelte';
  import { lib } from '../../lib/library.svelte';
  import { player } from '../../lib/player.svelte';
  import { ui, toast } from '../../lib/ui.svelte';
  import type { Track } from '../../lib/api';
  import { openMusicCover, refetchCovers } from '../../lib/musicart.svelte';
  import { fmtT } from '../../lib/util';
  import {
    DIMS, dimLabel, groupTracks, sortGroups, groupSub, albumArtist, albumKey,
    loadGroupSort, saveGroupSort, type Dim, type Group, type GroupSort,
  } from '../../lib/musicgroups';

  // 音乐库的分类浏览：卡片墙 → 点进去看这一组的歌（艺人页里还有他的专辑）
  let {
    tracks,
    dim,
    q = '',
    stack = $bindable([]),
  }: {
    /** 参与分类的歌曲（不含加密格式） */
    tracks: Track[];
    dim: Dim;
    /** 顶部筛选框：筛卡片名字 */
    q?: string;
    /** 打开的分组，最后一个是当前页；艺人页点专辑会再压一层 */
    stack?: { dim: Dim; key: string }[];
  } = $props();

  let gsort = $state<GroupSort>(loadGroupSort());
  $effect(() => saveGroupSort(gsort));

  const groups = $derived(groupTracks(tracks, dim));
  const shown = $derived.by(() => {
    const qq = q.trim().toLowerCase();
    const gs = qq ? groups.filter(g => (g.name + ' ' + g.sub).toLowerCase().includes(qq)) : groups;
    return sortGroups(gs, gsort, dim);
  });
  const unit = $derived(DIMS.find(d => d[0] === dim)![2]);

  // 当前打开的分组（按键重新找，媒体库变化后跟着更新）
  const top = $derived(stack.at(-1) ?? null);
  const cur = $derived.by(() => {
    if (!top) return null;
    const src = stack.length > 1 && stack[0].dim === 'artist' ? (findGroup(stack[0].dim, stack[0].key)?.tracks ?? []) : tracks;
    return groupTracks(src, top.dim).find(g => g.key === top.key) ?? null;
  });
  function findGroup(d: Dim, key: string): Group | undefined {
    return groupTracks(tracks, d).find(g => g.key === key);
  }
  // 打开的分组没了（歌都删了）：退回去
  $effect(() => { if (top && !cur) stack = stack.slice(0, -1); });

  // 艺人页：他的专辑
  const artistAlbums = $derived(cur && top?.dim === 'artist'
    ? groupTracks(cur.tracks, 'album').filter(g => !g.unknown).sort((a, b) => (b.tracks.find(t => t.year)?.year ?? 0) - (a.tracks.find(t => t.year)?.year ?? 0) || a.name.localeCompare(b.name, 'zh'))
    : []);
  const total = $derived(cur ? cur.tracks.reduce((n, t) => n + t.duration, 0) : 0);

  let root = $state<HTMLElement>();
  function open(d: Dim, key: string) {
    stack = [...stack, { dim: d, key }];
    queueMicrotask(() => root?.closest('.lib')?.scrollTo(0, 0));
  }
  /** 从歌曲跳到它的艺人 / 专辑：新开一层，不在艺人页里套 */
  function jump(d: Dim, key: string) {
    stack = [{ dim: d, key }];
    queueMicrotask(() => root?.closest('.lib')?.scrollTo(0, 0));
  }
  function back() {
    stack = stack.slice(0, -1);
  }

  // ---------- 播放 ----------
  function play(ts: Track[], i = 0, shuffle = false) {
    if (!ts.length) return toast('这一组还没有歌曲');
    if (shuffle && !player.shuffle) player.setMode('shuffle');
    player.playList(ts, shuffle ? Math.floor(Math.random() * ts.length) : i, 'library');
    if (i === 0) toast(`开始播放：${ts.length} 首`);
  }
  function groupEntries(g: Group, d: Dim): MenuEntry[] {
    const ids = g.tracks.map(t => t.id);
    return [
      { label: '打开', run: () => open(d, g.key) },
      { label: '播放', run: () => play(g.tracks) },
      { label: '随机播放', run: () => play(g.tracks, 0, true) },
      { label: '下一首播放', run: () => { player.playNext(g.tracks); toast(`已插到下一首：${ids.length} 首`); } },
      { label: '加入播放队列', run: () => { player.enqueue(g.tracks); toast(`已加入队列：${ids.length} 首`); } },
      { label: '加入歌单…', run: () => ui.open('playlist-add', ids) },
      { label: '转换格式…', run: () => { ui.convertPaths = g.tracks.map(t => t.path); ui.go('convert'); } },
      // 专辑：按这张专辑联网搜封面、整张一起换；其他分组可以批量重新获取
      ...(d === 'album' && g.tracks[0] ? [{ label: '更换封面…', run: () => openMusicCover(g.tracks[0].id, g.name) }] : []),
      { label: '重新获取封面', run: () => refetchCovers(ids) },
    ];
  }
  function cardMenu(e: MouseEvent, g: Group, d: Dim) {
    openMenu(e, g.name, groupEntries(g, d));
  }

  // ---------- 曲目 ----------
  /** 歌曲所属的艺人 / 专辑分组，用来“查看艺人”“查看专辑” */
  function jumpEntries(t: Track): MenuEntry[] {
    const out: MenuEntry[] = [];
    if (t.album && top?.dim !== 'album') out.push({ label: `查看专辑「${t.album}」`, run: () => jump('album', albumKey(t)) });
    const artists = groupTracks([t], 'artist').filter(g => !g.unknown && !(top?.dim === 'artist' && top.key === g.key));
    for (const a of artists.slice(0, 3)) out.push({ label: `查看艺人「${a.name}」`, run: () => jump('artist', a.key) });
    return out;
  }
  function trackMenu(e: MouseEvent, t: Track, list: Track[]) {
    openMenu(e, t.title, mediaEntries('track', t.id, t.title, [
      { label: '播放', run: () => play(list, list.indexOf(t)) },
      { label: '下一首播放', run: () => player.playNext([t]) },
      { label: '加入播放队列', run: () => player.enqueue([t]) },
      { label: '加入歌单…', run: () => ui.open('playlist-add', [t.id]) },
      { label: t.favorite ? '取消收藏' : '收藏', run: () => lib.toggleFav(t.id).then(v => toast(v ? '已收藏' : '已取消收藏')) },
      ...jumpEntries(t),
      { label: '转换格式…', run: () => { ui.convertPaths = [t.path]; ui.go('convert'); } },
      { label: '文件信息', run: () => ui.open('about', t) },
    ]));
  }

  const fmtTotal = (s: number) => {
    const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60);
    return h ? `${h} 小时 ${m} 分钟` : `${m} 分钟`;
  };
  const backLabel = $derived(stack.length > 1 ? `← 返回${findGroup(stack[stack.length - 2].dim, stack[stack.length - 2].key)?.name ?? ''}` : `← 返回${dimLabel(dim)}列表`);
  // 专辑分碟显示
  const multiDisc = $derived(top?.dim === 'album' && cur ? new Set(cur.tracks.map(t => t.disc_no ?? 1)).size > 1 : false);
</script>

<div class="mb" bind:this={root}>
  {#if cur && top}
    {@const d = top.dim}
    <button class="mp-chip" onclick={back}>{backLabel}</button>
    <div class="mb-head">
      <Cover cls="mb-art big {d === 'artist' ? 'round' : ''}" cover={cur.cover} name={cur.name} hue={cur.hue}>
        {#if d !== 'artist' && d !== 'album' && !cur.cover}<span class="mb-ico"><Icon name={d === 'folder' ? 'folder' : 'music'} /></span>{/if}
      </Cover>
      <div class="mb-info">
        <small>{dimLabel(d)}</small>
        <h2 title={cur.name}>{cur.name}</h2>
        {#if d === 'album' && !cur.unknown}<p>{albumArtist(cur.tracks)}</p>{/if}
        {#if d === 'folder'}<p class="mb-path" title={cur.key}>{cur.tracks[0] ? cur.tracks[0].path.replace(/[\\/][^\\/]*$/, '') : ''}</p>{/if}
        <div class="acts" style="margin:8px 0 12px">
          {#if d === 'artist' && artistAlbums.length}<span class="badge2">{artistAlbums.length} 张专辑</span>{/if}
          <span class="badge2">{cur.tracks.length} 首</span>
          <span class="badge2">{fmtTotal(total)}</span>
          {#if d === 'album' && cur.tracks.find(t => t.year)}<span class="badge2">{cur.tracks.find(t => t.year)!.year}</span>{/if}
          {#if cur.plays}<span class="badge2">播放 {cur.plays} 次</span>{/if}
        </div>
        <div class="acts">
          <button class="mp-btn" onclick={() => play(cur.tracks)}><Icon name="play" /> 播放全部</button>
          <button class="mp-btn ghost" onclick={() => play(cur.tracks, 0, true)}><Icon name="shuffle" /> 随机播放</button>
          <button class="mp-btn ghost" onclick={() => ui.open('playlist-add', cur.tracks.map(t => t.id))}><Icon name="plus" /> 加入歌单</button>
          <MoreButton floating={false} title="更多" onopen={e => openMenu(e, cur.name, groupEntries(cur, d).slice(1))} />
        </div>
      </div>
    </div>

    {#if artistAlbums.length}
      <h3 class="mb-h">专辑</h3>
      <div class="mp-grid mb-grid">
        {#each artistAlbums as g (g.key)}
          {@render card(g, 'album')}
        {/each}
      </div>
      <h3 class="mb-h">全部歌曲</h3>
    {/if}

    <div class="mb-list">
      {#each cur.tracks as t, i (t.id)}
        {#if multiDisc && (i === 0 || (cur.tracks[i - 1].disc_no ?? 1) !== (t.disc_no ?? 1))}
          <div class="grp">第 {t.disc_no ?? 1} 张碟</div>
        {/if}
        <div class="mp-row mrow" class:on={player.current?.id === t.id} role="button" tabindex="-1"
          onclick={() => play(cur.tracks, i)} oncontextmenu={e => trackMenu(e, t, cur.tracks)} onkeydown={() => {}}>
          <span class="mp-n">{String(d === 'album' && t.track_no ? t.track_no : i + 1).padStart(2, '0')}</span>
          <Cover cover={t.cover} name={t.album} hue={t.hue} />
          <span class="mp-t">{t.title}<small>{t.artist || '未知艺人'}</small></span>
          <span class="mp-al" style="white-space:nowrap;overflow:hidden;text-overflow:ellipsis">{t.album}</span>
          <span class="mp-fmt" class:lossless={t.lossless}>{t.format}{t.bit_depth && t.sample_rate && t.lossless ? ` ${t.bit_depth}/${Math.round(t.sample_rate / 1000)}` : ''}</span>
          <span class="pc">{t.play_count ? t.play_count + ' 次' : '未播放'}</span>
          <span class="mp-d">{fmtT(t.duration)}</span>
          <span class="ra">
            <button class:fav={t.favorite} title="收藏" onclick={e => { e.stopPropagation(); lib.toggleFav(t.id); }}><Icon name="heart" /></button>
            <button title="加入歌单" onclick={e => { e.stopPropagation(); ui.open('playlist-add', [t.id]); }}><Icon name="plus" /></button>
            <button title="更多" onclick={e => { e.stopPropagation(); trackMenu(e, t, cur.tracks); }}><Icon name="more" /></button>
          </span>
        </div>
      {/each}
    </div>
  {:else}
    <div class="mb-bar">
      <span class="engine-note">{shown.length}{q.trim() && shown.length !== groups.length ? ` / ${groups.length}` : ''} {unit} · 点卡片查看歌曲，卡片上的播放键直接播放整组</span>
      <select class="sel" bind:value={gsort}>
        <option value="name">{dim === 'year' ? '按年份' : '按名称'}</option>
        <option value="count">按歌曲数</option>
        <option value="plays">按播放次数</option>
        <option value="added">按最近添加</option>
      </select>
    </div>
    {#if !shown.length}
      <div class="empty">{q.trim() ? '没有匹配的' + dimLabel(dim) : '还没有可以分类的歌曲'}</div>
    {:else}
      <div class="mp-grid mb-grid" class:artists={dim === 'artist'}>
        {#each shown as g (g.key)}
          {@render card(g, dim)}
        {/each}
      </div>
    {/if}
  {/if}
</div>

{#snippet card(g: Group, d: Dim)}
  <div class="mp-card mb-card" class:round={d === 'artist'} role="button" tabindex="0" title={d === 'folder' ? g.sub : g.name}
    onclick={() => open(d, g.key)} onkeydown={e => e.key === 'Enter' && open(d, g.key)} oncontextmenu={e => cardMenu(e, g, d)}>
    <Cover cls="mb-art {d === 'artist' ? 'round' : ''}" cover={g.cover} name={g.name} hue={g.hue}>
      {#if !g.cover && d !== 'album'}<span class="mb-tag">{d === 'artist' ? g.name.slice(0, 1).toUpperCase() : g.name}</span>{/if}
      <button class="mb-play" title="播放" onclick={e => { e.stopPropagation(); play(g.tracks); }}><Icon name="play" /></button>
      {#if d !== 'artist'}<span class="mb-more"><MoreButton floating={false} onopen={e => cardMenu(e, g, d)} /></span>{/if}
    </Cover>
    <div class="mp-card-t">{g.name}</div>
    <div class="mp-card-s">{groupSub(g, d)}</div>
  </div>
{/snippet}

<style>
  .mb > .mp-chip:first-child { margin-top: 12px; }
  .mb-bar { display: flex; align-items: center; gap: 10px; justify-content: space-between; margin: 14px 2px 14px; }
  .mb-grid { grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); }
  .mb-card { content-visibility: auto; contain-intrinsic-size: auto 210px; }
  .mb-card.round { text-align: center; }
  .mb :global(.mb-art) { display: block; position: relative; aspect-ratio: 1; border-radius: var(--mp-radius); background-size: cover; background-position: center; overflow: hidden; box-shadow: 0 8px 24px rgba(0,0,0,.3); transition: transform .3s, box-shadow .3s; }
  .mb :global(.mb-art.round) { border-radius: 50%; }
  .mb-card:hover :global(.mb-art) { transform: translateY(-3px) scale(1.02); box-shadow: 0 14px 32px rgba(0,0,0,.45); }
  .mb-tag { position: absolute; inset: 0; display: grid; place-items: center; padding: 12px; text-align: center; color: #fff; font-weight: 700; font-size: 17px; line-height: 1.25; text-shadow: 0 2px 10px rgba(0,0,0,.5); overflow: hidden; word-break: break-all; }
  .round .mb-tag { font-size: 42px; }
  .mb-play { position: absolute; right: 10px; bottom: 10px; width: 38px; height: 38px; border-radius: 50%; border: 0; display: grid; place-items: center; background: var(--mp-accent); color: #fff; box-shadow: 0 6px 16px rgba(0,0,0,.4); opacity: 0; translate: 0 6px; transition: opacity .2s, translate .2s; }
  .round .mb-play { right: 50%; margin-right: -19px; bottom: 14px; }
  .mb-card:hover .mb-play, .mb-play:focus-visible { opacity: 1; translate: 0 0; }
  .mb-more { position: absolute; right: 8px; top: 8px; display: grid; place-items: center; width: 28px; height: 28px; border-radius: 50%; background: rgba(0,0,0,.45); color: #fff; opacity: 0; transition: opacity .2s; }
  .mb-card:hover .mb-more, .mb-more:focus-within { opacity: 1; }

  .mb-head { display: grid; grid-template-columns: 190px minmax(0, 1fr); gap: 24px; align-items: end; margin: 14px 0 8px; }
  .mb :global(.mb-art.big) { width: 190px; }
  .mb-ico { position: absolute; inset: 0; display: grid; place-items: center; font-size: 56px; color: #fff; opacity: .85; }
  .mb-info { min-width: 0; }
  .mb-info small { color: var(--mp-sub); font-size: 12px; letter-spacing: .08em; }
  .mb-info h2 { margin: 2px 0 4px; font-size: 30px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .mb-info p { margin: 0; color: var(--mp-sub); font-size: 14px; }
  .mb-path { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-size: 12.5px !important; }
  .mb-info .mp-btn { display: inline-flex; align-items: center; gap: 6px; }
  .mb-h { margin: 22px 2px 12px; font-size: 15px; }
  .mb-list { margin-top: 10px; }
  .mb-list .mrow { content-visibility: auto; contain-intrinsic-size: auto 46px; }
  @media (max-width: 720px) {
    .mb-head { grid-template-columns: 1fr; }
    .mb :global(.mb-art.big) { width: 140px; }
  }
</style>
