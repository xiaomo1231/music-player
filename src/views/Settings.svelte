<script lang="ts">
  import { onMount } from 'svelte';
  import { engine } from '../lib/engine.svelte';
  import { player } from '../lib/player.svelte';
  import { ui, toast } from '../lib/ui.svelte';
  import { invoke, inTauri } from '../lib/api';
  import { fmtSize } from '../lib/util';
  import { confirmDlg } from '../lib/items.svelte';
  import { lib } from '../lib/library.svelte';
  import {
    autostartGet,
    autostartSet,
    cacheClear,
    cacheSize,
    getSetting,
    setDesktopLyrics,
    setExplorerMenu,
    setSetting,
    setWallpaper,
    type CacheInfo,
  } from '../lib/shell.svelte';
  import HotkeyList from '../components/settings/HotkeyList.svelte';
  import LyricsSettings from '../components/settings/LyricsSettings.svelte';
  import FxSettings from '../components/settings/FxSettings.svelte';
  import ReaderSettings from '../components/settings/ReaderSettings.svelte';

  // 设置项的当前值（启动时从 settings 表读取）
  let watch = $state(true);
  let metaOnline = $state(false);
  let fade = $state(false);
  let useDefVol = $state(false);
  let defVol = $state(80);
  let closeToTray = $state(false);
  let autostart = $state(false);
  let autostartMin = $state(false);
  let wallpaper = $state(false);
  let wallBusy = $state(false);
  let lyrics = $state(false);
  let explorer = $state(false);
  let cache = $state<CacheInfo | null>(null);
  let clearCovers = $state(false);
  let clearing = $state(false);

  const inp = (e: Event) => (e.currentTarget as HTMLInputElement);

  async function refreshCache() {
    if (!inTauri) return;
    try {
      cache = await cacheSize();
    } catch (e) {
      toast('读取缓存大小失败：' + String(e));
    }
  }

  onMount(async () => {
    watch = await getSetting('watch', true);
    metaOnline = await getSetting('metadata_online', false);
    fade = await getSetting('fade', false);
    const dv = await getSetting<number | null>('default_volume', null);
    useDefVol = dv != null;
    defVol = dv ?? Math.round(player.vol * 100);
    closeToTray = await getSetting('close_to_tray', false);
    explorer = await getSetting('explorer_menu', false);
    lyrics = await getSetting('desktop_lyrics', false);
    if (inTauri) {
      const a = await autostartGet().catch(() => null);
      if (a) {
        autostart = a.enabled;
        autostartMin = a.minimized;
      }
      refreshCache();
    }
  });

  /** 普通开关：先改界面，再保存 */
  function flag(key: string, set: (v: boolean) => void) {
    return (e: Event) => {
      const v = inp(e).checked;
      set(v);
      setSetting(key, v);
    };
  }

  async function onWallpaper(e: Event) {
    const on = inp(e).checked;
    if (wallBusy) return;
    wallBusy = true;
    try {
      await setWallpaper(on);
      wallpaper = on;
      if (on) toast('壁纸模式已开启（非官方做法，部分系统版本可能不生效）');
    } catch (err) {
      inp(e).checked = false;
      wallpaper = false;
      toast('壁纸模式开启失败：' + String(err));
    } finally {
      wallBusy = false;
    }
  }

  async function onLyrics(e: Event) {
    const on = inp(e).checked;
    try {
      await setDesktopLyrics(on);
      lyrics = on;
    } catch (err) {
      inp(e).checked = !on;
      toast('桌面歌词：' + String(err));
    }
  }

  async function onExplorer(e: Event) {
    const on = inp(e).checked;
    try {
      await setExplorerMenu(on);
      explorer = on;
      toast(on ? '已加入资源管理器右键菜单' : '已移除资源管理器右键菜单');
    } catch (err) {
      inp(e).checked = !on;
      toast('设置失败：' + String(err));
    }
  }

  async function onAutostart(e: Event) {
    const on = inp(e).checked;
    try {
      await autostartSet(on, autostartMin);
      autostart = on;
    } catch (err) {
      inp(e).checked = !on;
      toast('开机启动设置失败：' + String(err));
    }
  }

  async function onAutostartMin(e: Event) {
    const v = inp(e).checked;
    try {
      await autostartSet(autostart, v);
      autostartMin = v;
    } catch (err) {
      inp(e).checked = !v;
      toast('保存失败：' + String(err));
    }
  }

  async function onDefVol(e: Event) {
    const on = inp(e).checked;
    useDefVol = on;
    await setSetting('default_volume', on ? defVol : null);
  }

  async function onDefVolValue(e: Event) {
    defVol = Number(inp(e).value);
    if (useDefVol) await setSetting('default_volume', defVol);
  }

  async function clearCache() {
    clearing = true;
    try {
      const freed = await cacheClear(clearCovers);
      toast(`已清理缓存，释放 ${fmtSize(freed)}`);
    } catch (e) {
      toast('清理失败：' + String(e));
    } finally {
      clearing = false;
      refreshCache();
    }
  }

  async function resetLibrary(full: boolean) {
    const ok = await confirmDlg(
      full ? '恢复初始状态' : '清空媒体库',
      full
        ? '清空媒体库的全部条目和记录（音乐、视频、番剧、漫画、小说、歌单、播放记录、扫描文件夹、转换和下载记录），并把所有设置恢复默认。硬盘上的媒体文件一个都不会删。AI 密钥保存在 Windows 凭据管理器里，不受影响。'
        : '清空媒体库的全部条目和记录（音乐、视频、番剧、漫画、小说、歌单、播放记录、扫描文件夹、转换和下载记录）。硬盘上的媒体文件一个都不会删，设置保留。清空后可以重新扫描你自己的文件夹。',
      { ok: full ? '恢复初始状态' : '清空媒体库', danger: true },
    );
    if (!ok) return;
    try {
      const n = await invoke<number>('library_reset', { full });
      player.pause();
      await lib.refresh();
      toast(`已清空媒体库（${n} 个文件记录）`);
      if (full) setTimeout(() => location.reload(), 600);
    } catch (e) {
      toast(String(e));
    }
  }

  function openDataDir() {
    if (!inTauri || !engine.dataDir) return;
    invoke('show_in_folder', { path: engine.dataDir }).catch(e => toast(String(e)));
  }
</script>

<section class="lib on set">
  <header>
    <div>
      <h2>设置</h2>
      <p>播放、媒体库、桌面行为和快捷键</p>
    </div>
  </header>

  <div class="set-grid">
    <!-- 播放 -->
    <div class="card">
      <h5>播放</h5>
      <div class="mp-opt" style="display:block">
        <div>
          播放引擎：{engine.mpv ? 'libmpv（全格式）' : 'WebView2（内置解码）'}
          <small>
            {#if engine.mpv}
              已启用 libmpv，APE、DSD、WMA 等格式也能播放。
            {:else}
              内置解码支持 MP3、FLAC、WAV、OGG、Opus、AAC、M4A、WebM 等常见格式；APE、DSD、WMA 需要 mpv 引擎。
              要启用 mpv，请把 libmpv-2.dll 放到程序所在目录后重新启动。
            {/if}
          </small>
        </div>
      </div>
      <label class="mp-opt">
        <div>启动时使用固定音量<small>关闭时沿用上次的音量</small></div>
        <input type="checkbox" class="mp-sw" checked={useDefVol} onchange={onDefVol} />
      </label>
      {#if useDefVol}
        <div class="mp-opt">
          <div>默认音量 {defVol}%</div>
          <input type="range" min="0" max="100" value={defVol} oninput={onDefVolValue} aria-label="默认音量" />
        </div>
      {/if}
      <label class="mp-opt">
        <div>淡入淡出<small>切歌和暂停时渐变音量（即将支持）</small></div>
        <input type="checkbox" class="mp-sw" checked={fade} onchange={flag('fade', v => (fade = v))} />
      </label>
    </div>

    <!-- 媒体库 -->
    <div class="card">
      <h5>媒体库</h5>
      <div class="mp-opt">
        <div>扫描目录<small>添加或移除要扫描的文件夹</small></div>
        <button class="mp-btn ghost" onclick={() => ui.open('scan')}>管理</button>
      </div>
      <label class="mp-opt">
        <div>实时监控<small>自动发现新加入的文件并入库</small></div>
        <input type="checkbox" class="mp-sw" checked={watch} onchange={flag('watch', v => (watch = v))} />
      </label>
      <label class="mp-opt">
        <div>联网补全元数据<small>用于补全歌曲、视频的信息和封面；默认关闭，开启后会联网</small></div>
        <input type="checkbox" class="mp-sw" checked={metaOnline} onchange={flag('metadata_online', v => (metaOnline = v))} />
      </label>
    </div>

    <!-- 外观与桌面 -->
    <div class="card">
      <h5>外观与桌面</h5>
      <div class="mp-opt">
        <div>OLED 防烧屏<small>纯黑背景、像素位移、自动变暗等</small></div>
        <button class="mp-btn ghost" onclick={() => ui.open('oled')}>设置</button>
      </div>
      <label class="mp-opt">
        <div>壁纸模式<small>把窗口嵌到桌面图标下方（非官方做法，开启时同时进入沉浸模式）</small></div>
        <input type="checkbox" class="mp-sw" checked={wallpaper} disabled={wallBusy} onchange={onWallpaper} />
      </label>
      <label class="mp-opt">
        <div>桌面歌词<small>在桌面上显示当前一行歌词，鼠标可以穿过它；快捷键 Ctrl + Alt + L</small></div>
        <input type="checkbox" class="mp-sw" checked={lyrics} onchange={onLyrics} />
      </label>
      <div class="mp-opt">
        <div>迷你播放器<small>置顶小窗，显示封面、歌词和基本控制</small></div>
        <button class="mp-btn ghost" onclick={() => ui.setMini(true)}>进入</button>
      </div>
      <label class="mp-opt">
        <div>开机启动<small>登录 Windows 后自动运行</small></div>
        <input type="checkbox" class="mp-sw" checked={autostart} disabled={!inTauri} onchange={onAutostart} />
      </label>
      <label class="mp-opt">
        <div>启动后最小化到托盘<small>开机启动时不弹出窗口</small></div>
        <input type="checkbox" class="mp-sw" checked={autostartMin} disabled={!autostart} onchange={onAutostartMin} />
      </label>
      <label class="mp-opt">
        <div>关闭窗口时最小化到托盘<small>点右上角关闭按钮后继续在托盘运行；从托盘右键退出</small></div>
        <input type="checkbox" class="mp-sw" checked={closeToTray} onchange={flag('close_to_tray', v => (closeToTray = v))} />
      </label>
    </div>

    <LyricsSettings />
    <FxSettings />
    <ReaderSettings />

    <!-- 快捷键 -->
    <div class="card card-wide">
      <h5>全局热键</h5>
      <p class="note">软件在后台或最小化时也有效。点“修改”后按下新的组合，按 Esc 取消；被其他软件占用的会标红。</p>
      <HotkeyList />
      <div class="rowbtns">
        <button class="mp-btn ghost" onclick={() => ui.open('keys')}>查看应用内快捷键</button>
      </div>
    </div>

    <!-- 系统集成 -->
    <div class="card">
      <h5>系统集成</h5>
      <label class="mp-opt">
        <div>资源管理器右键菜单<small>音频和视频文件右键增加“用极光播放器播放”“加入极光播放器媒体库”（只写当前用户，不影响其他用户）</small></div>
        <input type="checkbox" class="mp-sw" checked={explorer} disabled={!inTauri} onchange={onExplorer} />
      </label>
      <div class="mp-opt">
        <div>AI 服务<small>AI 字幕、自动获取和漫画翻译需要先填写服务</small></div>
        <button class="mp-btn ghost" onclick={() => ui.open('ai')}>配置</button>
      </div>
    </div>

    <!-- 存储 -->
    <div class="card">
      <h5>存储</h5>
      <div class="mp-opt">
        <div>数据目录<small>{engine.dataDir || '（仅桌面程序中可用）'}</small></div>
        <button class="mp-btn ghost" disabled={!inTauri || !engine.dataDir} onclick={openDataDir}>打开文件夹</button>
      </div>
      <div class="mp-opt">
        <div>
          缓存大小
          <small>缩略图 {cache ? fmtSize(cache.thumbs) : '—'}，封面 {cache ? fmtSize(cache.covers) : '—'}</small>
        </div>
        <button class="mp-btn ghost" disabled={!inTauri} onclick={refreshCache}>刷新</button>
      </div>
      <label class="mp-opt">
        <div>同时清理封面缓存<small>封面会在下次需要时重新生成；不影响媒体库和数据库</small></div>
        <input type="checkbox" class="mp-sw" checked={clearCovers} onchange={e => (clearCovers = inp(e).checked)} />
      </label>
      <div class="rowbtns">
        <button class="mp-btn" disabled={!inTauri || clearing} onclick={clearCache}>{clearing ? '清理中…' : '清理缓存'}</button>
      </div>
      <div class="mp-opt">
        <div>清空媒体库<small>删除库里所有条目和记录，从头开始；不删硬盘上的任何文件</small></div>
        <button class="mp-btn ghost" disabled={!inTauri} onclick={() => resetLibrary(false)}>清空媒体库…</button>
      </div>
      <div class="mp-opt">
        <div>恢复初始状态<small>清空媒体库，并把所有设置恢复默认</small></div>
        <button class="mp-btn ghost" disabled={!inTauri} onclick={() => resetLibrary(true)}>恢复初始状态…</button>
      </div>
    </div>

    <!-- 关于 -->
    <div class="card">
      <h5>关于</h5>
      <div class="kv">
        <span>版本</span><span>{engine.version || '—'}</span>
        <span>播放引擎</span><span>{engine.mpv ? 'libmpv' : 'WebView2'}</span>
      </div>
      <p class="note">开源组件：FFmpeg（音视频转换、截取封面，许可取决于构建配置，LGPL 或 GPL）；libmpv（播放引擎，LGPL 2.1 或更高）。使用这些组件时请保留其许可声明。</p>
    </div>
  </div>
</section>

<style>
  .set-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
    gap: 14px;
    align-items: start;
  }
  .card-wide {
    grid-column: 1 / -1;
  }
  .note {
    color: var(--mp-sub);
    font-size: 12.5px;
    line-height: 1.6;
    margin: 4px 0 8px;
  }
  input[type='range'] {
    flex: 1;
    min-width: 120px;
  }
  @media (max-width: 720px) {
    .set-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
