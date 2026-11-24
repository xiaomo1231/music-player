<script lang="ts">
  import Icon from './lib/Icon.svelte';
  import { ui, toast, type View } from './lib/ui.svelte';
  import { player } from './lib/player.svelte';
  import { lib } from './lib/library.svelte';
  import { invoke, inTauri, listen } from './lib/api';
  import { typing } from './lib/util';
  import Search from './components/Search.svelte';
  import PlayerBar from './components/PlayerBar.svelte';
  import MiniPlayer from './components/MiniPlayer.svelte';
  import NowPlaying from './views/NowPlaying.svelte';
  import FxBackground from './components/fx/Background.svelte';
  import Music from './views/Music.svelte';
  import Videos from './views/Videos.svelte';
  import Anime from './views/Anime.svelte';
  import Manga from './views/Manga.svelte';
  import Novels from './views/Novels.svelte';
  import Daily from './views/Daily.svelte';
  import Convert from './views/Convert.svelte';
  import AiSub from './views/AiSub.svelte';
  import Fetch from './views/Fetch.svelte';
  import Organize from './views/Organize.svelte';
  import Settings from './views/Settings.svelte';
  import ScanModal from './components/ScanModal.svelte';
  import EqModal from './components/EqModal.svelte';
  import OledModal from './components/OledModal.svelte';
  import MiniModal from './components/MiniModal.svelte';
  import AiModal from './components/AiModal.svelte';
  import PlaylistAddModal from './components/PlaylistAddModal.svelte';
  import KeysModal from './components/KeysModal.svelte';
  import FileInfoModal from './components/FileInfoModal.svelte';
  import ItemMenu from './components/ItemMenu.svelte';
  import UpdateModal from './components/UpdateModal.svelte';
  import CloseModal from './components/CloseModal.svelte';
  import EmImportModal from './components/em/ImportModal.svelte';
  import { takeEm, startEm } from './lib/em.svelte';
  import PluginInstallModal from './components/plugin/InstallModal.svelte';
  import PluginDetail from './components/plugin/PluginDetail.svelte';
  import { takePlugins, startPlugins } from './lib/plugin.svelte';
  import { updater } from './lib/updater.svelte';
  import VideoPlayer from './overlays/VideoPlayer.svelte';
  import NovelReader from './overlays/NovelReader.svelte';
  import ComicReader from './overlays/ComicReader.svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { onMount } from 'svelte';

  const nav: [View, string, string, string?][] = [
    ['music', 'music', '音乐'],
    ['video', 'video', '视频'],
    ['movie', 'film', '电影'],
    ['anime', 'anime', '番剧'],
    ['manga', 'list', '漫画'],
    ['novel', 'book', '小说'],
    ['daily', 'heart', '每日推荐', 'AI'],
  ];
  const tools: [View, string, string, string?][] = [
    ['convert', 'convert', '格式转换'],
    ['aisub', 'sub', 'AI 字幕', 'AI'],
    ['get', 'download', '自动获取', 'AI'],
    ['org', 'folder', '整理文件', 'AI'],
  ];

  // body 上的状态类，沿用原型的样式
  $effect(() => {
    const b = document.body;
    b.dataset.page = ui.view;
    b.classList.toggle('playing', player.playing);
    b.classList.toggle('immersive', ui.immersive);
    b.classList.toggle('mini', ui.mini);
    b.classList.toggle('idle', ui.idle);
    b.classList.toggle('dimmed', ui.dimmed);
    const o = ui.oled;
    b.classList.toggle('oled-black', o.black);
    b.classList.toggle('oled-drift', o.drift);
    b.classList.toggle('oled-static', o.stat);
    b.classList.toggle('oled-hide', o.hide);
    b.style.setProperty('--mp-dim', String(o.dimLv));
  });

  // 像素位移
  $effect(() => {
    const o = ui.oled;
    if (!o.shift) {
      document.body.style.translate = '';
      return;
    }
    const iv = setInterval(() => {
      const r = () => Math.round((Math.random() * 2 - 1) * 4);
      document.body.style.translate = `${r()}px ${r()}px`;
    }, o.shiftSec * 1000);
    return () => clearInterval(iv);
  });

  // 元素漂移：沿不规则路线每 15 秒移动 1 像素（约 ±12 像素范围），比逐帧动画省得多，防残影效果一样
  $effect(() => {
    if (!ui.oled.drift) {
      document.body.style.removeProperty('--drift-x');
      document.body.style.removeProperty('--drift-y');
      return;
    }
    const way = [[0, 0], [10, -6], [-8, 9], [7, 12], [-11, -5], [4, -10], [0, 0]];
    let x = 0, y = 0, k = 1;
    const iv = setInterval(() => {
      const [tx, ty] = way[k];
      x += Math.sign(tx - x);
      y += Math.sign(ty - y);
      if (x === tx && y === ty) k = (k + 1) % way.length;
      document.body.style.setProperty('--drift-x', x + 'px');
      document.body.style.setProperty('--drift-y', y + 'px');
    }, 15000);
    return () => clearInterval(iv);
  });

  // 无操作：隐藏控件 / 变暗
  let idleT: ReturnType<typeof setTimeout> | undefined, dimT: ReturnType<typeof setTimeout> | undefined;
  function wake() {
    ui.idle = false;
    ui.dimmed = false;
    clearTimeout(idleT);
    clearTimeout(dimT);
    idleT = setTimeout(() => (ui.idle = true), 3500);
    if (ui.oled.dim) dimT = setTimeout(() => (ui.dimmed = true), ui.oled.dimSec * 1000);
  }
  // 鼠标没动时，页面内容变化（歌词滚动、封面转动等）也会让 WebView 补发 mousemove，
  // 当成有操作的话沉浸模式下的播放条永远不会自动隐藏：只在指针位置真的变了时才算
  let lastX = -1, lastY = -1;
  function onMove(e: MouseEvent) {
    if (e.screenX === lastX && e.screenY === lastY) return;
    lastX = e.screenX;
    lastY = e.screenY;
    wake();
  }

  async function setImmersive(v?: boolean) {
    ui.immersive = v ?? !ui.immersive;
    if (ui.immersive) ui.view = 'now';
    if (inTauri) await getCurrentWindow().setFullscreen(ui.immersive).catch(() => {});
    else if (ui.immersive) document.documentElement.requestFullscreen?.().catch(() => {});
    else if (document.fullscreenElement) document.exitFullscreen();
  }

  async function setMini(v?: boolean) {
    ui.mini = v ?? !ui.mini;
    if (!inTauri) return;
    const w = getCurrentWindow();
    const { LogicalSize } = await import('@tauri-apps/api/dpi');
    if (ui.mini) {
      await w.setFullscreen(false).catch(() => {});
      ui.immersive = false;
      await w.setDecorations(false);
      // 先放开最小尺寸再缩小，否则会被主窗口的 900×600 下限卡住，留下一大片空白
      await w.setMinSize(new LogicalSize(340, 104)).catch(() => {});
      await w.setSize(new LogicalSize(360, 112));
      await w.setResizable(false).catch(() => {});
      await w.setAlwaysOnTop(true);
    } else {
      await w.setAlwaysOnTop(false);
      await w.setDecorations(true);
      await w.setResizable(true).catch(() => {});
      await w.setMinSize(new LogicalSize(900, 600)).catch(() => {});
      await w.setSize(new LogicalSize(1280, 800));
      await w.center().catch(() => {});
    }
  }
  ui.setImmersive = setImmersive;
  ui.setMini = setMini;

  function onKey(e: KeyboardEvent) {
    if (ui.videoId != null || ui.bookId != null || ui.comicId != null) return; // 覆盖层自己处理按键
    if (e.key === 'Escape') {
      if (ui.modal) ui.close();
      else if (ui.immersive) setImmersive(false);
      else if (ui.mini) setMini(false);
      return;
    }
    if (typing(e)) return;
    const k = e.key.toLowerCase();
    const c = e.ctrlKey || e.metaKey;
    if (k === ' ') {
      e.preventDefault();
      player.toggle();
    } else if (c && k === 'arrowright') player.next();
    else if (c && k === 'arrowleft') player.prev();
    else if (k === 'arrowright') player.seek(player.t + 5);
    else if (k === 'arrowleft') player.seek(player.t - 5);
    else if (k === 'arrowup') {
      e.preventDefault();
      player.setVol(player.vol + 0.05);
    } else if (k === 'arrowdown') {
      e.preventDefault();
      player.setVol(player.vol - 0.05);
    } else if (c && e.shiftKey && k === 'd') {
      if (player.current) ui.open('playlist-add', [player.current.id]);
    } else if (c && k === 'd') {
      e.preventDefault();
      if (player.current) lib.toggleFav(player.current.id).then(v => toast(v ? '已收藏' : '已取消收藏'));
    } else if (c && k === 'h') {
      e.preventDefault();
      player.toggleShuffle();
    } else if (c && k === 'r') {
      e.preventDefault();
      player.cycleMode();
    } else if (c && k === 'm') {
      e.preventDefault();
      setMini();
    } else if (c && k === 'f') {
      e.preventDefault();
      document.getElementById('gs')?.focus();
    } else if (c) return;
    else if (k === '/') {
      e.preventDefault();
      document.getElementById('gs')?.focus();
    } else if (k === 'f') setImmersive();
    else if (k === 'q') {
      ui.queueOpen = !ui.queueOpen;
      ui.view = 'now';
    } else if (k === 'e') ui.open('eq');
    else if (k === 'o') ui.open('oled');
    else if (k === 'm') player.toggleMute();
    else if (k === '?') ui.open('keys');
  }

  // 屏蔽 WebView 自带的右键菜单（刷新 / 另存为 / 检查等）；输入框里保留复制粘贴。
  // 开发时在控制台执行 localStorage.setItem('mp.devmenu','1') 可以恢复
  function onContextMenu(e: MouseEvent) {
    const t = e.target as HTMLElement | null;
    if (t?.closest('input, textarea, [contenteditable="true"], .selectable')) return;
    if (import.meta.env.DEV && localStorage.getItem('mp.devmenu') === '1') return;
    e.preventDefault();
  }

  // 拖放：文件或文件夹直接入库并播放 / 打开
  let dragging = $state(false);
  onMount(() => {
    wake();
    startEm();
    startPlugins();
    const offs: Promise<() => void>[] = [];
    if (inTauri) {
      offs.push(
        getCurrentWindow().onDragDropEvent(async ev => {
          const p = ev.payload;
          if (p.type === 'enter' || p.type === 'over') dragging = true;
          else if (p.type === 'leave') dragging = false;
          else if (p.type === 'drop') {
            dragging = false;
            if (ui.view === 'convert') return; // 转换页自己接收
            const paths = takePlugins(takeEm(p.paths)); // .em 规则文件、.mpx 插件包走安装确认
            if (!paths.length) return;
            const added = await invoke<[number, string][]>('add_files', { paths });
            await lib.refresh();
            const music = added.filter(a => a[1] === 'music').map(a => lib.byId(a[0])).filter(Boolean);
            if (music.length) {
              player.playList(music as never[], 0, 'drop');
              ui.view = 'now';
            } else if (added[0]) openItem(added[0][0], added[0][1]);
            toast(added.length ? `已加入媒体库：${added.length} 个文件` : '没有可识别的媒体文件');
          }
        }),
      );
      offs.push(listen<string>('scan://error', m => toast('扫描出错：' + m)));
      updater.autoCheck();
      // 首次启动：还没有扫描目录时打开向导
      invoke<{ id: number }[]>('list_roots').then(r => {
        if (!r.length) setTimeout(() => ui.open('scan'), 600);
      });
    }
    return () => offs.forEach(p => p.then(f => f()));
  });

  const openItem = (id: number, kind: string) => ui.openItem(id, kind);
  import { openPlaylistMenu } from './lib/playlists';

  async function openLocal() {
    if (!inTauri) return;
    const { open } = await import('@tauri-apps/plugin-dialog');
    const sel = await open({
      multiple: true,
      filters: [
        { name: '媒体文件', extensions: ['mp3', 'flac', 'm4a', 'wav', 'ogg', 'opus', 'ape', 'aac', 'wma', 'mp4', 'mkv', 'avi', 'mov', 'webm', 'txt', 'epub', 'cbz', 'zip', 'pdf', 'm3u', 'm3u8'] },
        { name: '所有文件', extensions: ['*'] },
      ],
    });
    if (!sel) return;
    const paths = (Array.isArray(sel) ? sel : [sel]) as string[];
    const lists = paths.filter(p => /\.m3u8?$/i.test(p));
    for (const l of lists) {
      const [, n] = await invoke<[number, number]>('import_playlist', { path: l });
      toast(`已导入歌单（${n} 首）`);
    }
    const files = paths.filter(p => !/\.m3u8?$/i.test(p));
    if (!files.length) return lib.refresh();
    const added = await invoke<[number, string][]>('add_files', { paths: files });
    await lib.refresh();
    const music = added.filter(a => a[1] === 'music').map(a => lib.byId(a[0])!).filter(Boolean);
    if (music.length) {
      player.playList(music, 0, 'open');
      ui.view = 'now';
    } else if (added[0]) openItem(added[0][0], added[0][1]);
  }

  async function newPlaylist() {
    ui.open('playlist-add', []);
  }
</script>

<svelte:window oncontextmenu={onContextMenu} onkeydown={onKey} onmousemove={onMove} onmousedown={wake} onkeydowncapture={wake} onwheel={wake} />

<FxBackground />
<aside data-static>
  <h1><i></i>极光播放器</h1>
  <button class:on={ui.view === 'now'} onclick={() => ui.go('now')}><Icon name="home" /><span>正在播放</span></button>
  <h6>媒体库</h6>
  {#each nav as [v, ic, label, badge]}
    <button class:on={ui.view === v} onclick={() => ui.go(v)}><Icon name={ic} /><span>{label}</span>{#if badge}<em>{badge}</em>{/if}</button>
  {/each}
  <h6>歌单</h6>
  <div id="plside">
    {#each lib.playlists as p (p.id)}
      <button class:on={ui.view === 'music' && ui.playlistId === p.id} onclick={() => { ui.playlistId = p.id; ui.go('music'); }}
        oncontextmenu={e => openPlaylistMenu(e, p)}>
        <Icon name="list" /><span>{p.name}</span><small>{p.count}</small>
      </button>
    {/each}
    <button onclick={newPlaylist}><Icon name="plus" /><span style="color:var(--mp-sub)">新建歌单</span></button>
  </div>
  <h6>工具</h6>
  {#each tools as [v, ic, label, badge]}
    <button class:on={ui.view === v} onclick={() => ui.go(v)}><Icon name={ic} /><span>{label}</span>{#if badge}<em>{badge}</em>{/if}</button>
  {/each}
  <button onclick={() => ui.open('scan')}><Icon name="scan" /><span>扫描媒体库</span></button>
  <h6>设置</h6>
  <button onclick={() => ui.open('ai')}><Icon name="gear" /><span>AI 服务</span></button>
  <button onclick={() => ui.open('oled')}><Icon name="oled" /><span>OLED 防烧屏</span></button>
  <button class:on={ui.view === 'settings'} onclick={() => ui.go('settings')}><Icon name="gear" /><span>设置</span></button>
  <button onclick={openLocal}><Icon name="folder" /><span>打开文件</span></button>
</aside>
<main>
  <div class="topbar" data-autohide>
    <Search />
    <span class="sp"></span>
    <button class="ib" title="扫描媒体库" onclick={() => ui.open('scan')}><Icon name="scan" /></button>
    <button class="ib" title="沉浸模式 (F)" onclick={() => setImmersive()}><Icon name="expand" /></button>
  </div>
  <div class="views">
    {#if ui.view === 'now'}<NowPlaying />
    {:else if ui.view === 'music'}<Music />
    {:else if ui.view === 'video'}<Videos />
    {:else if ui.view === 'movie'}<Videos movies />
    {:else if ui.view === 'anime'}<Anime />
    {:else if ui.view === 'manga'}<Manga />
    {:else if ui.view === 'novel'}<Novels />
    {:else if ui.view === 'daily'}<Daily />
    {:else if ui.view === 'convert'}<Convert />
    {:else if ui.view === 'aisub'}<AiSub />
    {:else if ui.view === 'get'}<Fetch />
    {:else if ui.view === 'org'}<Organize />
    {:else if ui.view === 'settings'}<Settings />
    {/if}
  </div>
</main>
<PlayerBar />
<MiniPlayer />

{#if ui.modal === 'scan'}<ScanModal />{/if}
{#if ui.modal === 'eq'}<EqModal />{/if}
{#if ui.modal === 'oled'}<OledModal />{/if}
{#if ui.modal === 'mini'}<MiniModal />{/if}
{#if ui.modal === 'ai'}<AiModal />{/if}
{#if ui.modal === 'playlist-add'}<PlaylistAddModal />{/if}
{#if ui.modal === 'keys'}<KeysModal />{/if}
{#if ui.modal === 'about'}<FileInfoModal />{/if}

{#if ui.videoId != null}<VideoPlayer />{/if}
{#if ui.bookId != null}<NovelReader />{/if}
{#if ui.comicId != null}<ComicReader />{/if}

<ItemMenu />
{#if updater.show}<UpdateModal />{/if}
<CloseModal />
<EmImportModal />
<PluginInstallModal />
<PluginDetail />

<div class="mp-dim"></div>
<div class="mp-refresh" class:run={ui.refreshRun > 0}></div>
<div class="mp-toast" class:show={ui.toastShow}>{ui.toastMsg}</div>
{#if dragging}<div class="drop-hint">松开即可加入媒体库并播放</div>{/if}
