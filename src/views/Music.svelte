<script lang="ts">
  import Icon from '../lib/Icon.svelte';
  import Cover from '../components/Cover.svelte';
  import { untrack } from 'svelte';
  import MoreButton from '../components/MoreButton.svelte';
  import BatchBar from '../components/BatchBar.svelte';
  import SelCheck from '../components/SelCheck.svelte';
  import { openMenu as popMenu, mediaEntries, ask, confirmDlg, pickImage, RESET, removeMany, deleteMany, type MenuEntry } from '../lib/items.svelte';
  import { Selection, noTextSelect, moveIds } from '../lib/select.svelte';
  import { openPlaylistMenu, renamePlaylist, exportPlaylist, deletePlaylist } from '../lib/playlists';
  import { coverUrl } from '../lib/api';
  import { lib } from '../lib/library.svelte';
  import { player } from '../lib/player.svelte';
  import { ui, toast } from '../lib/ui.svelte';
  import { invoke, inTauri, type Track } from '../lib/api';
  import { fmtT, coverCSS } from '../lib/util';
  import { smartLists, customLists, matchRules, type SmartDef } from '../lib/smart.svelte';
  import SmartRuleModal from '../components/SmartRuleModal.svelte';

  let filter = $state<string>('all'); // all / smart:<key> / pl:<id>
  let group = $state<'none' | 'artist' | 'album' | 'format' | 'folder' | 'year'>('none');
  let sort = $state<'def' | 'plays' | 'added' | 'title' | 'duration'>('def');
  let q = $state('');
  let plTracks = $state<number[]>([]);
  let ruleEdit = $state<SmartDef | null | undefined>(undefined);

  // 侧栏点了歌单就切过来
  $effect(() => {
    if (ui.playlistId != null) {
      filter = 'pl:' + ui.playlistId;
    }
  });
  $effect(() => {
    void lib.version;
    if (filter.startsWith('pl:') && inTauri) invoke<number[]>('playlist_items', { id: +filter.slice(3) }).then(v => (plTracks = v));
  });

  const allSmart = $derived([...smartLists.filter(s => !customLists.hidden.includes(s.key)), ...customLists.list]);
  const hiddenSmart = $derived(smartLists.filter(s => customLists.hidden.includes(s.key)));
  const curPl = $derived(filter.startsWith('pl:') ? lib.playlists.find(p => p.id === +filter.slice(3)) : null);

  const list = $derived.by(() => {
    let l: Track[];
    if (curPl) l = plTracks.map(id => lib.byId(id)).filter((t): t is Track => !!t);
    else if (filter.startsWith('smart:')) {
      const def = allSmart.find(s => s.key === filter.slice(6));
      l = def ? def.pick(lib.tracks) : lib.tracks;
    } else l = lib.tracks;
    const qq = q.trim().toLowerCase();
    if (qq) l = l.filter(t => (t.title + t.artist + t.album + t.format + t.genre).toLowerCase().includes(qq));
    const s = [...l];
    if (sort === 'plays') s.sort((a, b) => b.play_count - a.play_count);
    else if (sort === 'added') s.sort((a, b) => b.added_at - a.added_at);
    else if (sort === 'title') s.sort((a, b) => a.title.localeCompare(b.title, 'zh'));
    else if (sort === 'duration') s.sort((a, b) => b.duration - a.duration);
    return s;
  });

  const keyOf = (t: Track): string => {
    switch (group) {
      case 'artist': return t.album_artist || t.artist || '未知艺人';
      case 'album': return t.album || '未知专辑';
      case 'format': return t.format || '未知格式';
      case 'year': return t.year ? String(t.year) : '未知年份';
      case 'folder': return t.path.replace(/[\\/][^\\/]*$/, '');
      default: return '';
    }
  };

  type Item = { h: string; tracks: Track[] } | { t: Track; n: number };
  const items = $derived.by(() => {
    if (group === 'none') return list.map((t, i) => ({ t, n: i + 1 }) as Item);
    const m = new Map<string, Track[]>();
    for (const t of list) {
      const k = keyOf(t);
      if (!m.has(k)) m.set(k, []);
      m.get(k)!.push(t);
    }
    const keys = [...m.keys()].sort((a, b) => (group === 'year' ? b.localeCompare(a) : a.localeCompare(b, 'zh')));
    const out: Item[] = [];
    for (const k of keys) {
      out.push({ h: k, tracks: m.get(k)! });
      m.get(k)!.forEach((t, i) => out.push({ t, n: i + 1 }));
    }
    return out;
  });

  // 简单虚拟列表：行高固定，只渲染可见部分
  const ROW = 46, HEAD = 40;
  let scroller: HTMLElement | null = $state(null);
  let listTop = $state(0);
  let scrollTop = $state(0);
  let viewH = $state(800);
  const offsets = $derived.by(() => {
    const o: number[] = [];
    let y = 0;
    for (const it of items) { o.push(y); y += 'h' in it ? HEAD : ROW; }
    o.push(y);
    return o;
  });
  const range = $derived.by(() => {
    const top = scrollTop - listTop - 400, bot = scrollTop - listTop + viewH + 400;
    let a = 0, b = items.length;
    let lo = 0, hi = items.length;
    while (lo < hi) { const m = (lo + hi) >> 1; if (offsets[m + 1] < top) lo = m + 1; else hi = m; }
    a = lo;
    lo = a; hi = items.length;
    while (lo < hi) { const m = (lo + hi) >> 1; if (offsets[m] <= bot) lo = m + 1; else hi = m; }
    b = lo;
    return [a, b];
  });
  let listEl: HTMLDivElement | undefined = $state();
  function onScroll() {
    if (!scroller) return;
    scrollTop = scroller.scrollTop;
    viewH = scroller.clientHeight;
    listTop = listEl ? listEl.offsetTop : 0;
  }
  $effect(() => { void items; void sel.on; onScroll(); });

  // ---------- 多选 ----------
  const sel = new Selection();
  /** 列表里歌曲的显示顺序（Shift 连选、全选、批量操作都按这个顺序） */
  const order = $derived(items.flatMap(it => ('t' in it ? [it.t.id] : [])));
  const byId = $derived(new Map(lib.tracks.map(t => [t.id, t])));
  // 换列表时退出多选；列表内容变了（删除、移除）时去掉已不存在的
  $effect(() => { void filter; untrack(() => sel.clear()); });
  $effect(() => { const o = order; untrack(() => sel.prune(o)); });

  const picked = () => sel.ids(order).map(id => byId.get(id)).filter((t): t is Track => !!t);

  async function favMany(ts: Track[], favorite: boolean) {
    await invoke('item_favorite_many', { ids: ts.map(t => t.id), favorite });
    for (const t of ts) t.favorite = favorite;
    toast(favorite ? `已收藏 ${ts.length} 首` : `已取消收藏 ${ts.length} 首`);
  }
  async function plRemoveMany(ids: number[]) {
    const pl = curPl;
    if (!pl) return;
    await invoke('playlist_remove', { id: pl.id, files: ids });
    const gone = new Set(ids);
    plTracks = plTracks.filter(x => !gone.has(x));
    lib.refreshPlaylists();
    sel.clear();
    toast(`已从歌单移除 ${ids.length} 首`);
  }
  /** 批量操作（批量栏和右键菜单共用） */
  function batchEntries(): MenuEntry[] {
    const ts = picked();
    const ids = ts.map(t => t.id);
    const pl = curPl;
    return [
      { label: '播放', run: () => { player.playList(ts, 0, filter); toast(`开始播放：${ts.length} 首`); } },
      { label: '下一首播放', run: () => player.playNext(ts) },
      { label: '加入队列', run: () => player.enqueue(ts) },
      { label: '加入歌单…', run: () => ui.open('playlist-add', ids) },
      ...(ts.some(t => !t.favorite) ? [{ label: '收藏', run: () => favMany(ts, true) }] : []),
      ...(ts.some(t => t.favorite) ? [{ label: '取消收藏', run: () => favMany(ts, false) }] : []),
      { label: '转换格式…', run: () => { ui.convertPaths = ts.map(t => t.path); ui.go('convert'); } },
      ...(pl ? [{ label: '从这个歌单移除', run: () => plRemoveMany(ids) }] : []),
      'sep',
      { label: '从媒体库移除…', run: async () => { if (await removeMany('track', ids)) sel.clear(); } },
      { label: '删除文件…', danger: true, run: async () => { if (await deleteMany('track', ids)) sel.clear(); } },
    ];
  }
  function rowMenu(e: MouseEvent, t: Track) {
    if (sel.has(t.id) && sel.size > 1) popMenu(e, `已选 ${sel.size} 首`, batchEntries());
    else openMenu(e, t);
  }
  function rowClick(e: MouseEvent, t: Track) {
    if (dragged) { dragged = false; return; }
    if (!sel.click(e, t.id, order)) playFrom(t);
  }

  function playFrom(t: Track) {
    player.playList(list, list.indexOf(t), filter);
  }
  function playGroup(tracks: Track[], shuffle = false) {
    if (!tracks.length) return toast('这一组还没有歌曲');
    if (shuffle && !player.shuffle) player.toggleShuffle();
    player.playList(tracks, shuffle ? Math.floor(Math.random() * tracks.length) : 0, filter);
    toast(`开始播放：${tracks.length} 首`);
  }
  function openMenu(e: MouseEvent, t: Track) {
    const pl = curPl;
    popMenu(e, t.title, mediaEntries('track', t.id, t.title, [
      { label: '播放', run: () => playFrom(t) },
      { label: '下一首播放', run: () => player.playNext([t]) },
      { label: '加入播放队列', run: () => player.enqueue([t]) },
      { label: '加入歌单…', run: () => ui.open('playlist-add', [t.id]) },
      ...(pl ? [{ label: '从这个歌单移除', run: async () => { await invoke('playlist_remove', { id: pl.id, files: [t.id] }); plTracks = plTracks.filter(x => x !== t.id); lib.refreshPlaylists(); toast('已从歌单移除'); } }] : []),
      { label: t.favorite ? '取消收藏' : '收藏', run: () => lib.toggleFav(t.id).then(v => toast(v ? '已收藏' : '已取消收藏')) },
      { label: '转换格式…', run: () => { ui.convertPaths = [t.path]; ui.go('convert'); } },
      { label: '文件信息', run: () => ui.open('about', t) },
    ]));
  }

  // 智能列表卡片的“⋯”：内置的只能隐藏；自定义的可以改名、改规则、换封面、删除
  function smartMenu(e: MouseEvent, s: SmartDef) {
    const play = { label: '播放', run: () => playGroup(s.pick(lib.tracks)) };
    if (!s.custom) {
      popMenu(e, s.name, [play, 'sep', { label: '隐藏这个列表', run: async () => { await customLists.setHidden(s.key, true); if (filter === 'smart:' + s.key) setFilter('all'); toast('已隐藏，可以在“自定义智能列表”卡片里恢复'); } }]);
      return;
    }
    const raw = customLists.get(s.key);
    if (!raw) return;
    popMenu(e, s.name, [
      play,
      'sep',
      { label: '重命名…', run: async () => { const v = await ask('重命名智能列表', s.name); if (typeof v === 'string' && v.trim()) await customLists.save({ ...raw, name: v.trim() }); } },
      { label: '编辑规则…', run: () => (ruleEdit = s) },
      { label: '更换封面…', run: async () => { const p = await pickImage(); if (p) { const c = await invoke<string>('item_save_image', { path: p }); await customLists.save({ ...raw, cover: c }); } } },
      { label: '恢复默认封面', run: () => customLists.save({ ...raw, cover: null }) },
      'sep',
      { label: '删除这个列表…', danger: true, run: async () => { if (await confirmDlg('删除智能列表', `删除「${s.name}」？只删除这个列表的规则，歌曲不受影响。`, { ok: '删除', danger: true })) { await customLists.remove(s.key); if (filter === 'smart:' + s.key) setFilter('all'); toast('已删除'); } } },
    ]);
  }
  void RESET;

  async function plAct(a: string) {
    const p = curPl;
    if (!p) return;
    if (a === 'play') playGroup(list);
    if (a === 'rename') renamePlaylist(p);
    if (a === 'export') exportPlaylist(p);
    if (a === 'del') {
      await deletePlaylist(p);
      if (!lib.playlists.some(x => x.id === p.id)) filter = 'all';
    }
  }

  // 歌单内拖动排序（指针拖动，不用 HTML5 拖放：窗口开着文件拖放时 WebView 里的 HTML5 拖放不可靠）
  // 拖已选中的行时，所有选中的歌一起移动
  const canReorder = $derived(!!curPl && group === 'none' && sort === 'def' && !q.trim());
  let drag = $state<{ ids: number[]; y: number; gap: number } | null>(null);
  let dragged = false;
  function dragStart(e: PointerEvent, t: Track) {
    if (!canReorder || e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey) return;
    if ((e.target as HTMLElement).closest('button, .selck')) return;
    const y0 = e.clientY;
    const ids = sel.has(t.id) && sel.size > 1 ? sel.ids(plTracks) : [t.id];
    const move = (ev: PointerEvent) => {
      if (!drag && Math.abs(ev.clientY - y0) < 6) return;
      const r = listEl!.getBoundingClientRect();
      const gap = Math.max(0, Math.min(list.length, Math.round((ev.clientY - r.top) / ROW)));
      drag = { ids, y: ev.clientY, gap };
      // 靠近上下边缘时自动滚动
      const sr = scroller!.getBoundingClientRect();
      if (ev.clientY < sr.top + 60) scroller!.scrollTop -= 14;
      else if (ev.clientY > sr.bottom - 40) scroller!.scrollTop += 14;
    };
    const up = () => {
      removeEventListener('pointermove', move);
      removeEventListener('pointerup', up);
      const d = drag;
      drag = null;
      if (d) {
        dragged = true;
        setTimeout(() => (dragged = false), 0);
        dropAt(d.ids, d.gap);
      }
    };
    addEventListener('pointermove', move);
    addEventListener('pointerup', up);
  }
  async function dropAt(ids: number[], gap: number) {
    if (!curPl) return;
    const next = moveIds(plTracks, ids, gap);
    if (next.every((id, i) => id === plTracks[i])) return;
    plTracks = next;
    await invoke('playlist_reorder', { id: curPl.id, files: next });
    toast(ids.length > 1 ? `已移动 ${ids.length} 首` : '已调整顺序');
  }

  function setFilter(f: string) {
    filter = f;
    ui.playlistId = f.startsWith('pl:') ? +f.slice(3) : null;
  }
</script>

<svelte:window onkeydown={e => sel.keys(e, () => order)} />

<section class="lib on" class:multi={sel.on} bind:this={scroller} onscroll={onScroll}>
  <header>
    <div><h2>音乐</h2><p>共 {lib.tracks.length} 首{list.length !== lib.tracks.length ? `，当前显示 ${list.length} 首` : ''} · 点卡片上的播放键直接播放整组</p></div>
    <input class="tin" placeholder="在音乐库里筛选" bind:value={q} style="width:200px" />
    <select class="sel" bind:value={group}>
      <option value="none">不分组</option><option value="artist">按艺人分组</option><option value="album">按专辑分组</option>
      <option value="format">按格式分组</option><option value="folder">按文件夹分组</option><option value="year">按年份分组</option>
    </select>
    <select class="sel" bind:value={sort}>
      <option value="def">默认排序</option><option value="plays">按播放次数</option><option value="added">按添加时间</option><option value="title">按标题</option><option value="duration">按时长</option>
    </select>
    {#if lib.tracks.length}
      <button class="mp-chip msel" class:on={sel.on} title="多选后批量播放、加入歌单、移除或删除（也可以 Ctrl / Shift 点击，Ctrl+A 全选）"
        onclick={() => (sel.on ? sel.clear() : (sel.mode = true))}><Icon name="check" /> {sel.on ? '退出多选' : '多选'}</button>
    {/if}
  </header>

  {#if !lib.tracks.length}
    <div class="empty"><b>音乐库还是空的</b>添加一个音乐文件夹，或者直接把文件拖进窗口<br /><button class="mp-btn" onclick={() => ui.open('scan')}>扫描媒体库</button></div>
  {:else}
    <div class="smart">
      {#each allSmart as s (s.key)}
        {@const n = s.pick(lib.tracks).length}
        <div class="sc" class:on={filter === 'smart:' + s.key} style:background-image={coverCSS(s.hue, true)} role="button" tabindex="-1"
          onclick={() => setFilter(filter === 'smart:' + s.key ? 'all' : 'smart:' + s.key)} onkeydown={() => {}}
          oncontextmenu={e => smartMenu(e, s)} style:background-size="cover">
          {#if s.cover}<span class="sc-img" style:background-image={`url("${coverUrl(s.cover)}")`}></span>{/if}
          <div class="sc-txt"><b>{s.name}</b><span class="sc-sub">{n} 首 · {s.desc}</span></div>
          <div class="sc-acts">
            <button class="sc-play" title="立即播放" onclick={e => { e.stopPropagation(); playGroup(s.pick(lib.tracks)); }}><Icon name="play" /></button>
            <span class="sc-more"><MoreButton floating={false} title="更多" onopen={e => smartMenu(e, s)} /></span>
          </div>
        </div>
      {/each}
      <div class="sc sc-add" role="button" tabindex="-1" onclick={() => (ruleEdit = null)} onkeydown={() => {}}>
        <span style="text-align:center"><Icon name="plus" size="20px" /><br />自定义智能列表</span>
        {#if hiddenSmart.length}
          <span class="sc-hidden">已隐藏：{#each hiddenSmart as h}<button class="mp-chip" title="恢复显示" onclick={e => { e.stopPropagation(); customLists.setHidden(h.key, false); }}>{h.name} ↺</button>{/each}</span>
        {/if}
      </div>
    </div>

    <div class="chips">
      <button class="mp-chip" class:on={filter === 'all'} onclick={() => setFilter('all')}>全部歌曲</button>
      {#each allSmart as s (s.key)}<button class="mp-chip" class:on={filter === 'smart:' + s.key} onclick={() => setFilter('smart:' + s.key)}>{s.name}</button>{/each}
      <span style="width:1px;height:18px;background:var(--edge);margin:0 4px"></span>
      {#each lib.playlists as p (p.id)}<button class="mp-chip" class:on={filter === 'pl:' + p.id} onclick={() => setFilter('pl:' + p.id)} oncontextmenu={e => openPlaylistMenu(e, p)}><Icon name="list" /> {p.name} · {p.count}</button>{/each}
      <button class="mp-chip" onclick={() => ui.open('playlist-add', [])}>＋ 新建歌单</button>
    </div>

    {#if sel.on}
      <BatchBar {sel} unit="首" total={order.length} onall={() => sel.selectAll(order)} entries={batchEntries()}
        hint={curPl && canReorder ? '单击勾选，Shift 连选，Ctrl+A 全选；拖动选中的歌可以一起调整顺序' : undefined} />
    {:else if curPl}
      <div class="plbar">
        <b>{curPl.name}</b><span class="badge2">{curPl.count} 首</span>
        <button class="mp-chip" onclick={() => plAct('play')}><Icon name="play" /> 播放</button>
        <button class="mp-chip" onclick={() => plAct('rename')}>重命名</button>
        <button class="mp-chip" onclick={() => plAct('export')}>导出 .m3u8</button>
        <button class="mp-chip" onclick={() => plAct('del')}>删除歌单</button>
        <MoreButton floating={false} title="更多" onopen={e => openPlaylistMenu(e, curPl!)} />
        <span class="engine-note">{canReorder ? '拖动歌曲可以调整顺序，选中多首可以一起拖' : '默认排序、不分组、不筛选时可以拖动调整顺序'}</span>
      </div>
    {:else}
      <div class="plbar"><button class="mp-chip" onclick={() => playGroup(list)}><Icon name="play" /> 播放全部</button><button class="mp-chip" onclick={() => playGroup(list, true)}><Icon name="shuffle" /> 随机播放</button></div>
    {/if}

    <div bind:this={listEl} class="vlist" class:dragging={!!drag} style:height="{offsets[items.length]}px">
      {#if drag}
        <div class="dropline" style:top="{drag.gap * ROW - 1}px"></div>
        <div class="dragtip" style:top="{drag.y}px">移动 {drag.ids.length} 首</div>
      {/if}
      {#each items.slice(range[0], range[1]) as it, j (('h' in it ? 'h:' + it.h : it.t.id) + ':' + (range[0] + j))}
        {@const top = offsets[range[0] + j]}
        {#if 'h' in it}
          <div class="grp" style="position:absolute;left:0;right:0;top:{top}px;height:{HEAD}px;margin:0;padding:14px 6px 0">
            {it.h} · {it.tracks.length} 首
            <button class="mp-chip" style="padding:2px 10px" onclick={() => playGroup(it.tracks)}><Icon name="play" /> 播放</button>
          </div>
        {:else}
          {@const t = it.t}
          {@const checked = sel.has(t.id)}
          <div class="mp-row mrow" class:on={player.current?.id === t.id} class:picked={checked} class:moving={drag?.ids.includes(t.id)}
            style="position:absolute;left:0;right:0;top:{top}px;height:{ROW}px"
            role="button" tabindex="-1" onpointerdown={e => dragStart(e, t)} onmousedown={noTextSelect}
            ondblclick={() => playFrom(t)} onclick={e => rowClick(e, t)} oncontextmenu={e => rowMenu(e, t)} onkeydown={() => {}}>
            <span class="mp-n"><span class="num">{String(it.n).padStart(2, '0')}</span>
              <SelCheck {checked} onclick={e => (e.shiftKey ? sel.range(order, t.id, true) : sel.toggle(t.id))} /></span>
            <Cover cover={t.cover} name={t.album} hue={t.hue} />
            <span class="mp-t">{t.title}<small>{t.artist || '未知艺人'}</small></span>
            <span class="mp-al" style="white-space:nowrap;overflow:hidden;text-overflow:ellipsis">{t.album}</span>
            <span class="mp-fmt" class:lossless={t.lossless}>{t.format}{t.bit_depth && t.sample_rate && t.lossless ? ` ${t.bit_depth}/${Math.round(t.sample_rate / 1000)}` : ''}</span>
            <span class="pc">{t.play_count ? t.play_count + ' 次' : '未播放'}</span>
            <span class="mp-d">{fmtT(t.duration)}</span>
            <span class="ra">
              <button class:fav={t.favorite} title="收藏 (Ctrl+D)" onclick={e => { e.stopPropagation(); lib.toggleFav(t.id); }}><Icon name="heart" /></button>
              <button title="加入歌单" onclick={e => { e.stopPropagation(); ui.open('playlist-add', [t.id]); }}><Icon name="plus" /></button>
              {#if curPl}<button title="移出歌单" onclick={async e => { e.stopPropagation(); await invoke('playlist_remove', { id: curPl.id, files: [t.id] }); plTracks = plTracks.filter(x => x !== t.id); lib.refreshPlaylists(); }}><Icon name="close" /></button>{/if}
              <button title="更多" onclick={e => { e.stopPropagation(); rowMenu(e, t); }}><Icon name="more" /></button>
            </span>
          </div>
        {/if}
      {/each}
    </div>
  {/if}
</section>

{#if ruleEdit !== undefined}<SmartRuleModal def={ruleEdit} onclose={() => (ruleEdit = undefined)} />{/if}

<style>
  /* 智能列表卡片：固定高度、顶部对齐；文字在左上，按钮锚在右下角，不参与文字流 */
  .smart { align-items: start; }
  .sc { height: 100px; min-height: 0; padding: 0; }
  .sc .sc-img { position: absolute; inset: 0; background-size: cover; background-position: center; opacity: .55; }
  .sc .sc-txt { position: absolute; left: 14px; top: 12px; right: 86px; }
  .sc .sc-txt b { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sc .sc-sub { display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; margin-top: 3px; line-height: 1.35; }
  .sc .sc-acts { position: absolute; right: 10px; bottom: 10px; display: flex; gap: 6px; align-items: center; }
  .sc .sc-more { display: grid; place-items: center; width: 30px; height: 30px; border-radius: 50%; background: rgba(0, 0, 0, .45); color: #fff; opacity: 0; transition: opacity .2s; }
  .sc:hover .sc-more, .sc .sc-more:focus-within { opacity: 1; }
  .sc.sc-add { display: grid; place-items: center; text-align: center; background: var(--glass); padding: 8px; }
  .sc .sc-hidden { display: flex; flex-wrap: wrap; gap: 4px; justify-content: center; font-size: 11.5px; margin-top: 4px; color: var(--mp-sub); align-items: center; max-height: 44px; overflow: hidden; }
  .sc .sc-hidden .mp-chip { padding: 2px 8px; font-size: 11px; }

  /* 多选：序号位置悬停时变成勾选框，多选状态下一直显示 */
  .msel { display: inline-flex; align-items: center; gap: 5px; }
  .mrow .mp-n { display: grid; align-items: center; justify-items: start; }
  .mrow .mp-n > :global(*) { grid-area: 1 / 1; }
  .mrow .mp-n :global(.selck) { opacity: 0; }
  .mrow:hover .mp-n :global(.selck), .multi .mrow .mp-n :global(.selck) { opacity: 1; }
  .mrow:hover .num, .multi .mrow .num { opacity: 0; }
  .mrow.picked { background: color-mix(in srgb, var(--mp-accent) 15%, transparent); }
  .mrow.picked:hover { background: color-mix(in srgb, var(--mp-accent) 22%, transparent); }
  .mrow.moving { opacity: .45; }
  .vlist.dragging, .vlist.dragging .mrow { cursor: grabbing; }
  .dropline { position: absolute; left: 0; right: 0; height: 2px; background: var(--mp-accent); z-index: 3; pointer-events: none; box-shadow: 0 0 8px var(--mp-accent); }
  .dragtip { position: fixed; left: 50%; translate: -50% -140%; z-index: 20; pointer-events: none; font-size: 12px; padding: 3px 10px; border-radius: 99px; background: var(--mp-accent); color: #0b0b10; font-weight: 600; }
  /* 窄窗口：保留序号 / 勾选框这一列 */
  @media (max-width: 980px) {
    .mrow { grid-template-columns: 2em 2.6em minmax(0, 1fr) 3.2em 4.6em; }
    .mrow .mp-n { display: grid; }
  }
</style>
