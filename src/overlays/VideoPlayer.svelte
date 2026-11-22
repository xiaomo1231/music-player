<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import Icon from '../lib/Icon.svelte';
  import { ui } from '../lib/ui.svelte';
  import { invoke, inTauri, fileUrl, type Video } from '../lib/api';
  import { clamp, fmtT, fmtSize, typing } from '../lib/util';
  import { parseVtt, pickSub, type Cue, type MediaInfo, type SubItem } from '../lib/media.svelte';
  import Danmaku, { type DmItem } from '../components/media/Danmaku.svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { engine } from '../lib/engine.svelte';
  import { toast } from '../lib/ui.svelte';

  const RATES = [0.5, 0.75, 1, 1.25, 1.5, 2];
  const FRAME = 1 / 25; // 没有读到帧率时按 25 帧算
  const SKIP = 85; // Ctrl + → 跳过片头的秒数
  const VOL_KEY = 'mp.videoVol';

  function loadDmOn(): boolean {
    try {
      return localStorage.getItem('mp.video.danmaku') !== '0';
    } catch {
      return true;
    }
  }
  const loadVol = () => {
    try {
      const v = Number(localStorage.getItem(VOL_KEY));
      return localStorage.getItem(VOL_KEY) != null && v >= 0 && v <= 1 ? v : 0.8;
    } catch {
      return 0.8;
    }
  };

  let overlay = $state<HTMLElement | null>(null);
  let video = $state<HTMLVideoElement | null>(null);
  let barEl = $state<HTMLElement | null>(null);
  let cur = $state<Video | null>(null);
  let srcUrl = $state('');
  let info = $state<MediaInfo | null>(null);
  let err = $state('');
  let time = $state(0);
  let dur = $state(0);
  let paused = $state(true);
  let rate = $state(1);
  let holdRate = $state<number | null>(null);
  let vol = $state(loadVol());
  let muted = $state(false);
  let subs = $state<SubItem[]>([]);
  let subIdx = $state(-1);
  let subVisible = $state(true);
  let subOffset = $state(0); // 正数 = 字幕提前显示
  let subScale = $state(1);
  let cues = $state<Cue[]>([]);
  // 弹幕（视频旁边有 .danmaku.ass 时才有），开关记在本机
  const DM_KEY = 'mp.video.danmaku';
  let dm = $state<DmItem[]>([]);
  let dmOn = $state(loadDmOn());
  let idle = $state(false);
  let helpOpen = $state(false);
  let infoOpen = $state(false);
  let osd = $state('');
  let osdShow = $state(false);
  let fit = $state<'contain' | 'cover' | 'fill'>('contain');
  let turn = $state(0);
  let ab = $state<{ a: number | null; b: number | null }>({ a: null, b: null });
  let top = $state(false);
  let fsOn = $state(false);
  let hoverT = $state<number | null>(null);
  let hoverX = $state(0);

  // 非响应式的内部状态
  let loadNo = 0;
  let resumeAt = 0;
  let osdT: ReturnType<typeof setTimeout> | undefined;
  let idleT: ReturnType<typeof setTimeout> | undefined;
  let saveT: ReturnType<typeof setInterval> | undefined;
  let keyT: ReturnType<typeof setTimeout> | undefined;
  let rewindT: ReturnType<typeof setInterval> | undefined;
  let keyHold: 'r' | 'l' | null = null;
  let mouseT: ReturnType<typeof setTimeout> | undefined;
  let clickT: ReturnType<typeof setTimeout> | undefined;
  let mouseHolding = false;
  let pressing = false;
  let scrubbing = false;
  const subCache = new Map<string, Cue[]>();

  const list = $derived(ui.videoList);
  const idx = $derived(ui.videoId == null ? -1 : list.indexOf(ui.videoId));
  const prevId = $derived(idx > 0 ? list[idx - 1] : null);
  const nextId = $derived(idx >= 0 && idx < list.length - 1 ? list[idx + 1] : null);
  const progress = $derived(dur > 0 ? clamp(time / dur) : 0);
  const effRate = $derived(holdRate ?? rate);
  const showNext = $derived(nextId != null && dur > 0 && dur - time <= 30);
  const showSkip = $derived(dur > 600 && time < 150);
  const activeCues = $derived(
    subVisible && subIdx >= 0 ? cues.filter(c => c.start <= time + subOffset && time + subOffset < c.end) : [],
  );

  // ---------- 提示 ----------

  function osdMsg(m: string, sticky = false) {
    osd = m;
    osdShow = true;
    clearTimeout(osdT);
    if (!sticky) osdT = setTimeout(() => (osdShow = false), 1000);
  }

  function wake() {
    idle = false;
    clearTimeout(idleT);
    idleT = setTimeout(() => (idle = true), 3000);
  }

  // ---------- 进度与存档 ----------

  function saveNow() {
    if (!cur) return;
    const t = video && srcUrl ? video.currentTime : time;
    const d = video && srcUrl && isFinite(video.duration) ? video.duration : dur;
    if (!(t > 0) && !(d > 0)) return;
    invoke('save_video_progress', { id: cur.id, position: t, duration: d > 0 ? d : 0 }).catch(() => {});
  }

  function seek(t: number) {
    if (!video) return;
    const v = clamp(t, 0, dur > 0 ? dur : Infinity);
    video.currentTime = v;
    time = v;
  }

  function seekBy(d: number) {
    seek((video?.currentTime ?? time) + d);
    osdMsg(`${d > 0 ? '+' : '−'}${Math.abs(d)} 秒`);
  }

  function seekPercent(k: number) {
    seek((dur * k) / 10);
    osdMsg(`跳到 ${k * 10}%`);
  }

  // ---------- 播放控制 ----------

  function togglePlay() {
    if (!video || !srcUrl) return;
    if (video.paused) {
      void video.play().catch(() => {});
      osdMsg('播放');
    } else {
      video.pause();
      osdMsg('暂停');
    }
  }

  function setRate(r: number) {
    rate = r;
    osdMsg(`速度 ${r}×`);
  }

  function stepRate(d: number) {
    const i = RATES.indexOf(rate) >= 0 ? RATES.indexOf(rate) : RATES.indexOf(1);
    setRate(RATES[clamp(i + d, 0, RATES.length - 1)]);
  }

  function setVol(v: number) {
    vol = clamp(v, 0, 1);
    muted = false;
    osdMsg(`音量 ${Math.round(vol * 100)}%`);
  }

  function toggleMute() {
    muted = !muted;
    osdMsg(muted ? '已静音' : `音量 ${Math.round(vol * 100)}%`);
  }

  function frameStep(d: number) {
    if (!video || !srcUrl) return;
    video.pause();
    seek(video.currentTime + d * FRAME);
    osdMsg(d > 0 ? '下一帧' : '上一帧');
  }

  /** 按住方向键：右键 3 倍速，左键快退；短按是 5 秒 */
  function startKeyHold(dir: 'r' | 'l') {
    keyHold = dir;
    clearTimeout(keyT);
    keyT = setTimeout(() => {
      keyT = undefined;
      if (dir === 'r') {
        holdRate = 3;
        osdMsg('3 倍速中，松开恢复', true);
      } else {
        rewindT = setInterval(() => seek((video?.currentTime ?? time) - 0.6), 100);
        osdMsg('快速倒退中，松开恢复', true);
      }
    }, 320);
  }

  function endKeyHold(dir: 'r' | 'l') {
    if (keyHold !== dir) return;
    keyHold = null;
    if (keyT) {
      clearTimeout(keyT);
      keyT = undefined;
      seekBy(dir === 'r' ? 5 : -5);
      return;
    }
    if (dir === 'r') holdRate = null;
    else {
      clearInterval(rewindT);
      rewindT = undefined;
    }
    osdShow = false;
  }

  function stopHolds() {
    clearTimeout(keyT);
    keyT = undefined;
    clearInterval(rewindT);
    rewindT = undefined;
    keyHold = null;
    holdRate = null;
    clearTimeout(mouseT);
    mouseHolding = false;
    pressing = false;
  }

  // ---------- 集数 ----------

  function goEpisode(id: number | null) {
    if (id == null) return;
    saveNow();
    ui.videoId = id;
  }

  function onEnded() {
    if (video) time = video.duration || time;
    saveNow();
    if (nextId != null) goEpisode(nextId);
    else osdMsg('播放完毕');
  }

  // ---------- 加载 ----------

  async function load(id: number) {
    const no = ++loadNo;
    err = '';
    info = null;
    subs = [];
    subIdx = -1;
    cues = [];
    time = 0;
    dur = 0;
    paused = true;
    srcUrl = '';
    if (!inTauri) {
      err = '不在桌面程序中运行，无法读取本地视频。';
      return;
    }
    let v: Video | null;
    try {
      v = await invoke<Video | null>('get_video', { id });
    } catch (e) {
      if (no === loadNo) err = String(e);
      return;
    }
    if (no !== loadNo) return;
    if (!v) {
      err = '找不到这个视频（可能已从媒体库移除）。';
      return;
    }
    cur = v;
    resumeAt = v.watched ? 0 : v.position;
    srcUrl = fileUrl(v.path);
    // 媒体信息与字幕列表在后台读取，不阻塞播放
    invoke<MediaInfo>('media_probe', { id })
      .then(r => {
        if (no === loadNo) info = r;
      })
      .catch(() => {});
    invoke<DmItem[]>('media_danmaku', { id })
      .then(list => {
        if (no === loadNo) dm = list;
      })
      .catch(() => {});
    invoke<SubItem[]>('media_subtitles', { id })
      .then(list => {
        if (no !== loadNo) return;
        subs = list;
        const i = pickSub(list);
        if (i >= 0) void selectSub(i);
      })
      .catch(() => {});
  }

  function unload() {
    saveNow();
    stopHolds();
    video?.pause();
    srcUrl = '';
    cur = null;
    info = null;
    subs = [];
    cues = [];
    dm = [];
    err = '';
    helpOpen = false;
    infoOpen = false;
    ab = { a: null, b: null };
    if (fsOn) void setFs(false);
  }

  $effect(() => {
    const id = ui.videoId;
    untrack(() => {
      if (id != null) void load(id);
      else unload();
    });
  });

  // 打开时每 5 秒存一次进度
  $effect(() => {
    if (ui.videoId == null) return;
    saveT = setInterval(saveNow, 5000);
    return () => clearInterval(saveT);
  });

  // 音量与倍速同步到 video 元素
  $effect(() => {
    if (video) {
      video.volume = vol;
      video.muted = muted;
    }
    try {
      localStorage.setItem(VOL_KEY, String(vol));
    } catch {
      /* 忽略 */
    }
  });
  $effect(() => {
    if (video) video.playbackRate = effRate;
  });

  function onMeta() {
    if (!video) return;
    dur = isFinite(video.duration) ? video.duration : 0;
    video.playbackRate = effRate;
    if (resumeAt > 5 && resumeAt < dur - 10) {
      video.currentTime = resumeAt;
      time = resumeAt;
      osdMsg(`从 ${fmtT(resumeAt)} 继续`);
    }
    resumeAt = 0;
  }

  function onTime() {
    if (!video) return;
    time = video.currentTime;
    if (ab.a != null && ab.b != null && video.currentTime >= ab.b) video.currentTime = ab.a;
  }

  function onDuration() {
    if (video && isFinite(video.duration)) dur = video.duration;
  }

  function onError() {
    if (!srcUrl || !video) return;
    const code = video.error?.code;
    err =
      code === 4
        ? '这个视频的格式或编码暂不被内置播放器支持（例如 HEVC、RMVB，或部分 MKV 的音视频编码）。'
        : `播放失败：${video.error?.message || '未知错误'}`;
  }

  /** 用独立的 mpv 窗口播放（网页播放器解不了的格式） */
  async function playInMpv() {
    if (!cur) return;
    const id = cur.id, start = cur.position;
    try {
      await invoke('mpv_play_video', { id, start });
      toast('已在 mpv 窗口中播放，关闭窗口后会记住进度');
      close();
    } catch (e) {
      toast('mpv 播放失败：' + e);
    }
  }

  function toConvert() {
    if (!cur) return;
    ui.convertPaths = [cur.path];
    ui.go('convert');
    ui.videoId = null;
  }

  // ---------- 字幕 ----------

  async function selectSub(i: number) {
    subIdx = i;
    cues = [];
    if (i < 0 || !cur) return;
    const s = subs[i];
    if (!s) return;
    const key = `${cur.id}:${s.kind}:${s.path ?? ''}:${s.stream ?? ''}`;
    const hit = subCache.get(key);
    if (hit) {
      cues = hit;
      return;
    }
    const no = loadNo;
    try {
      const args = s.kind === 'external' ? { path: s.path } : { video: cur.path, stream: s.stream };
      const text = await invoke<string>('media_sub_vtt', args);
      if (no !== loadNo || subIdx !== i) return;
      const parsed = parseVtt(text);
      subCache.set(key, parsed);
      cues = parsed;
      if (!parsed.length) osdMsg('这条字幕里没有可显示的文字');
    } catch {
      if (no === loadNo && subIdx === i) {
        subIdx = -1;
        osdMsg('这条字幕无法显示（可能是图形字幕，如 PGS / VobSub）');
      }
    }
  }

  function toggleDanmaku() {
    if (!dm.length) {
      osdMsg('这个视频没有弹幕');
      return;
    }
    dmOn = !dmOn;
    try {
      localStorage.setItem(DM_KEY, dmOn ? '1' : '0');
    } catch {
      // 存不了就只在本次生效
    }
    osdMsg(dmOn ? '弹幕：开' : '弹幕：关');
  }

  function cycleSub() {
    if (!subs.length) {
      osdMsg('这个视频没有字幕');
      return;
    }
    const pos = (subIdx + 2) % (subs.length + 1); // 0 = 关闭
    const ni = pos - 1;
    void selectSub(ni);
    osdMsg(ni < 0 ? '字幕：关' : `字幕：${subs[ni].label}`);
  }

  function shiftSub(d: number) {
    subOffset = Math.round((subOffset + d) * 10) / 10;
    osdMsg(subOffset === 0 ? '字幕已对齐' : `字幕${subOffset > 0 ? '提前' : '延后'} ${Math.abs(subOffset).toFixed(1)} 秒`);
  }

  function zoomSub(d: number) {
    subScale = Math.round(clamp(subScale + d, 0.5, 2.5) * 10) / 10;
    osdMsg(`字幕大小 ${Math.round(subScale * 100)}%`);
  }

  // ---------- 截图 / 画面 ----------

  async function shot(withSub: boolean) {
    if (!video || !cur) return;
    try {
      const w = video.videoWidth;
      const h = video.videoHeight;
      if (!w || !h) {
        osdMsg('画面还没有就绪');
        return;
      }
      const cv = document.createElement('canvas');
      cv.width = w;
      cv.height = h;
      const g = cv.getContext('2d');
      if (!g) throw new Error('无法创建画布');
      g.drawImage(video, 0, 0, w, h);
      if (withSub && activeCues.length) {
        const size = Math.round(h * 0.045 * subScale);
        g.font = `${size}px sans-serif`;
        g.textAlign = 'center';
        g.fillStyle = '#fff';
        g.strokeStyle = '#000';
        g.lineWidth = Math.max(2, size / 8);
        const lines = activeCues.flatMap(c => c.text.split('\n'));
        lines.forEach((t, i) => {
          const y = h * 0.88 - (lines.length - 1 - i) * size * 1.3;
          g.strokeText(t, w / 2, y);
          g.fillText(t, w / 2, y);
        });
      }
      const url = cv.toDataURL('image/png');
      const path = await invoke<string>('media_save_screenshot', { videoPath: cur.path, pngBase64: url, at: time });
      osdMsg(`已截图：${path.split(/[\\/]/).pop() ?? path}`);
    } catch {
      osdMsg('截图失败');
    }
  }

  function cycleFit() {
    fit = fit === 'contain' ? 'cover' : fit === 'cover' ? 'fill' : 'contain';
    osdMsg({ contain: '画面：适应', cover: '画面：填充裁切', fill: '画面：拉伸' }[fit]);
  }

  function rotate() {
    turn = (turn + 90) % 360;
    osdMsg(`旋转 ${turn}°`);
  }

  function cycleAB() {
    if (ab.a == null) {
      ab = { a: time, b: null };
      osdMsg(`A-B 循环：起点 ${fmtT(time)}，再按 R 设终点`);
    } else if (ab.b == null) {
      if (time <= ab.a + 0.5) {
        osdMsg('终点要在起点之后');
        return;
      }
      const a = ab.a;
      ab = { a, b: time };
      osdMsg(`A-B 循环：${fmtT(a)} → ${fmtT(time)}`);
    } else {
      ab = { a: null, b: null };
      osdMsg('已取消 A-B 循环');
    }
  }

  // ---------- 窗口 ----------

  async function isFs(): Promise<boolean> {
    if (inTauri) return getCurrentWindow().isFullscreen().catch(() => false);
    return !!document.fullscreenElement;
  }

  async function setFs(on: boolean) {
    if (inTauri) await getCurrentWindow().setFullscreen(on).catch(() => {});
    else if (on) overlay?.requestFullscreen?.().catch(() => {});
    else if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    fsOn = on;
  }

  async function toggleFs() {
    await setFs(!(await isFs()));
  }

  async function togglePip() {
    if (!video || !srcUrl) return;
    try {
      if (document.pictureInPictureElement) await document.exitPictureInPicture();
      else if (document.pictureInPictureEnabled) await video.requestPictureInPicture();
      else osdMsg('当前环境不支持画中画');
    } catch {
      osdMsg('无法进入画中画');
    }
  }

  async function toggleTop() {
    if (!inTauri) return;
    top = !top;
    await getCurrentWindow().setAlwaysOnTop(top).catch(() => {});
    osdMsg(top ? '窗口已置顶' : '已取消置顶');
  }

  function close() {
    ui.videoId = null;
  }

  // ---------- 鼠标 ----------

  const CONTROLS = '.mp-vbar,.mp-vtop,.mp-vhelp,.vinfo,.mp-verr,.mp-next,.mp-skip';

  function onStageDown(e: PointerEvent) {
    if (e.button !== 0 || (e.target as HTMLElement).closest(CONTROLS)) return;
    pressing = true;
    mouseHolding = false;
    clearTimeout(mouseT);
    mouseT = setTimeout(() => {
      mouseHolding = true;
      holdRate = 2;
      osdMsg('2 倍速中，松开恢复', true);
    }, 320);
  }

  function onStageUp(e: PointerEvent) {
    if (!pressing) return;
    pressing = false;
    clearTimeout(mouseT);
    if (mouseHolding) {
      mouseHolding = false;
      holdRate = null;
      osdShow = false;
      return;
    }
    // 单击播放 / 暂停；两次点击的位置决定双击动作
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const x = (e.clientX - r.left) / r.width;
    if (clickT) {
      clearTimeout(clickT);
      clickT = undefined;
      if (x < 1 / 3) seekBy(-10);
      else if (x > 2 / 3) seekBy(10);
      else void toggleFs();
    } else {
      clickT = setTimeout(() => {
        clickT = undefined;
        togglePlay();
      }, 240);
    }
  }

  function onStageLeave() {
    pressing = false;
    clearTimeout(mouseT);
    if (mouseHolding) {
      mouseHolding = false;
      holdRate = null;
      osdShow = false;
    }
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    setVol(vol + (e.deltaY < 0 ? 0.05 : -0.05));
  }

  function barT(e: PointerEvent): number {
    const r = barEl!.getBoundingClientRect();
    return clamp((e.clientX - r.left) / r.width) * dur;
  }

  function barDown(e: PointerEvent) {
    if (!dur || !barEl) return;
    scrubbing = true;
    barEl.setPointerCapture(e.pointerId);
    seek(barT(e));
  }

  function barMove(e: PointerEvent) {
    if (!barEl || !dur) return;
    hoverX = e.clientX - barEl.getBoundingClientRect().left;
    hoverT = barT(e);
    if (scrubbing) seek(hoverT);
  }

  function barUp() {
    scrubbing = false;
  }

  function barLeave() {
    if (!scrubbing) hoverT = null;
  }

  // ---------- 键盘 ----------

  function onKey(e: KeyboardEvent) {
    if (ui.videoId == null || typing(e)) return;
    const k = e.key;
    const key = k.length === 1 ? k.toLowerCase() : k;

    if (e.ctrlKey || e.metaKey) {
      switch (key) {
        case 's':
          void shot(true);
          break;
        case 'ArrowRight':
          seek((video?.currentTime ?? time) + SKIP);
          osdMsg(`跳过 ${SKIP} 秒`);
          break;
        case '=':
        case '+':
          zoomSub(0.1);
          break;
        case '-':
          zoomSub(-0.1);
          break;
        case 'a':
          cycleFit();
          break;
        case 'r':
          rotate();
          break;
        case 'z':
        case 'x':
          osdMsg('音频延迟调整需要 mpv 播放引擎');
          break;
        default:
          return;
      }
      e.preventDefault();
      return;
    }
    if (e.altKey) return;

    switch (key) {
      case ' ':
        togglePlay();
        break;
      case 'ArrowRight':
        if (!e.repeat) startKeyHold('r');
        break;
      case 'ArrowLeft':
        if (!e.repeat) startKeyHold('l');
        break;
      case 'ArrowUp':
        setVol(vol + 0.05);
        break;
      case 'ArrowDown':
        setVol(vol - 0.05);
        break;
      case 'm':
        toggleMute();
        break;
      case '[':
        stepRate(-1);
        break;
      case ']':
        stepRate(1);
        break;
      case '\\':
        setRate(1);
        break;
      case ',':
        frameStep(-1);
        break;
      case '.':
        frameStep(1);
        break;
      case 'j':
        seekBy(-10);
        break;
      case 'l':
        seekBy(10);
        break;
      case 'PageUp':
        goEpisode(prevId);
        break;
      case 'PageDown':
        goEpisode(nextId);
        break;
      case 'c':
        cycleSub();
        break;
      case 'd':
        toggleDanmaku();
        break;
      case 'v':
        subVisible = !subVisible;
        osdMsg(subVisible ? '字幕已显示' : '字幕已隐藏');
        break;
      case 'z':
        shiftSub(0.1);
        break;
      case 'x':
        shiftSub(-0.1);
        break;
      case 'a':
        osdMsg(info?.audio.length ? '切换音轨需要 mpv 播放引擎' : '没有可切换的音轨');
        break;
      case 's':
        void shot(false);
        break;
      case 'r':
        cycleAB();
        break;
      case 'f':
      case 'Enter':
        void toggleFs();
        break;
      case 'p':
        void togglePip();
        break;
      case 't':
        void toggleTop();
        break;
      case 'i':
      case 'Tab':
        infoOpen = !infoOpen;
        break;
      case '?':
      case '/':
        helpOpen = !helpOpen;
        break;
      case 'Escape':
        if (helpOpen) helpOpen = false;
        else if (infoOpen) infoOpen = false;
        else
          void (async () => {
            if (await isFs()) await setFs(false);
            else close();
          })();
        break;
      default:
        if (/^[0-9]$/.test(key)) seekPercent(Number(key));
        else return;
    }
    e.preventDefault();
  }

  function onKeyUp(e: KeyboardEvent) {
    if (ui.videoId == null || typing(e)) return;
    if (e.key === 'ArrowRight') {
      endKeyHold('r');
      e.preventDefault();
    } else if (e.key === 'ArrowLeft') {
      endKeyHold('l');
      e.preventDefault();
    }
  }

  onDestroy(() => {
    clearTimeout(osdT);
    clearTimeout(idleT);
    clearTimeout(clickT);
    stopHolds();
  });

  const trackLabel = (a: { lang: string; title: string; codec: string }) =>
    [a.lang || '未知语言', a.title, a.codec].filter(Boolean).join(' · ');
</script>

<svelte:window onkeydown={onKey} onkeyup={onKeyUp} />

<div
  class="mp-over"
  class:open={ui.videoId != null}
  class:idle={idle && !paused}
  class:help={helpOpen}
  bind:this={overlay}
  onmousemove={wake}
  role="dialog"
  aria-label="视频播放器"
  tabindex="-1"
>
  <div
    class="mp-vstage"
    role="presentation"
    onpointerdown={onStageDown}
    onpointerup={onStageUp}
    onpointerleave={onStageLeave}
    onwheel={onWheel}
  >
    <!-- 字幕由自己渲染（支持延迟与大小），不用 <track> -->
    <!-- svelte-ignore a11y_media_has_caption -->
    <video
      bind:this={video}
      class="mp-vid"
      src={srcUrl || undefined}
      playsinline
      autoplay
      style:object-fit={fit}
      style:transform="rotate({turn}deg)"
      onloadedmetadata={onMeta}
      ontimeupdate={onTime}
      ondurationchange={onDuration}
      onplay={() => (paused = false)}
      onpause={() => (paused = true)}
      onended={onEnded}
      onerror={onError}
    ></video>

    <Danmaku {video} items={dm} on={dmOn} />

    <div class="mp-sub" style:font-size={`calc(clamp(16px, 2.4vw, 30px) * ${subScale})`}>
      {#each activeCues as c, i (i)}
        <div>{c.text}</div>
      {/each}
    </div>

    {#if err}
      <div class="mp-verr">
        <p class="t">无法播放</p>
        <p>{err}</p>
        <p class="hint">{engine.mpv ? '内置播放器解不了这个格式，可以交给 mpv 播放，或者转换为 MP4。' : '内置播放器只能直接播放 MP4、WebM 等常见格式。HEVC、RMVB 等需要 mpv 播放引擎；现在可以先转换为 MP4 再看。'}</p>
        <div class="acts">
          {#if cur && engine.mpv}<button class="mp-btn" onclick={playInMpv}>用 mpv 播放</button>{/if}
          {#if cur}<button class="mp-btn" onclick={toConvert}>转换为 MP4</button>{/if}
          <button class="mp-btn ghost" onclick={close}>关闭</button>
        </div>
      </div>
    {/if}

    <div class="mp-vtop">
      <button class="vclose" onclick={close} aria-label="关闭">←</button>
      <b class="ttl">{cur?.title ?? ''}</b>
      {#if info}
        <span class="mp-tag">{info.codec || '未知编码'}</span>
        {#if info.width}<span class="mp-tag">{info.width}×{info.height}</span>{/if}
      {/if}
      <span class="grow"></span>
      <button class="vtxt" onclick={() => (helpOpen = !helpOpen)} title="快捷键 (?)">⌨</button>
    </div>

    <div class="mp-vbar">
      <div
        class="mp-vprog"
        bind:this={barEl}
        style="--p: {progress}"
        role="slider"
        tabindex="-1"
        aria-label="播放进度"
        aria-valuemin="0"
        aria-valuemax={Math.round(dur)}
        aria-valuenow={Math.round(time)}
        onpointerdown={barDown}
        onpointermove={barMove}
        onpointerup={barUp}
        onpointercancel={barUp}
        onpointerleave={barLeave}
      >
        <i></i>
        {#if hoverT != null}<span class="mp-vtip" style:left="{hoverX}px">{fmtT(hoverT)}</span>{/if}
      </div>

      <div class="mp-vrow">
        <button onclick={togglePlay} title="播放 / 暂停 (空格)"><Icon name={paused ? 'play' : 'pause'} /></button>
        <span class="tm">{fmtT(time)} / {fmtT(dur)}</span>
        {#if ab.a != null}<span class="mp-tag">A-B {fmtT(ab.a)}{ab.b != null ? ' → ' + fmtT(ab.b) : ''}</span>{/if}
        <span class="grow"></span>
        <span class="vol" title="音量 (↑ ↓ / 滚轮)">
          <button onclick={toggleMute} title="静音 (M)"><Icon name="vol" /></button>
          <input
            type="range"
            min="0"
            max="1"
            step="0.05"
            value={muted ? 0 : vol}
            oninput={e => setVol(Number(e.currentTarget.value))}
            aria-label="音量"
          />
        </span>
        <label class="sel">速度
          <select value={String(rate)} onchange={e => setRate(Number(e.currentTarget.value))}>
            {#each RATES as r (r)}<option value={String(r)}>{r}</option>{/each}
          </select>
        </label>
        {#if dm.length}
          <button class="dmb" class:on={dmOn} onclick={toggleDanmaku} title="弹幕开关 (D)">弹</button>
        {/if}
        <label class="sel">字幕
          <select value={String(subIdx)} onchange={e => void selectSub(Number(e.currentTarget.value))}>
            <option value="-1">关闭</option>
            {#each subs as s, i (i)}<option value={String(i)}>{s.label}</option>{/each}
          </select>
        </label>
        <button onclick={() => goEpisode(prevId)} disabled={prevId == null} title="上一集 (PgUp)"><Icon name="prev" /></button>
        <button onclick={() => goEpisode(nextId)} disabled={nextId == null} title="下一集 (PgDn)"><Icon name="next" /></button>
        <button onclick={togglePip} title="画中画 (P)"><Icon name="mini" /></button>
        <button onclick={() => shot(false)} title="截图 (S)"><Icon name="image" /></button>
        <button onclick={toggleFs} title="全屏 (F)"><Icon name="expand" /></button>
      </div>
    </div>

    {#if showNext}
      <button class="mp-next" onclick={() => goEpisode(nextId)}>下一集 ▸</button>
    {/if}
    {#if showSkip}
      <button class="mp-skip" onclick={() => seekBy(SKIP)}>跳过片头 {SKIP} 秒</button>
    {/if}

    {#if infoOpen}
      <div class="vinfo">
        <b>媒体信息</b>
        {#if cur}<p>文件：{cur.path}（{fmtSize(cur.size)}）</p>{/if}
        {#if info}
          <p>时长 {fmtT(info.duration)}{info.width ? ` · ${info.width}×${info.height}` : ''}{info.codec ? ` · ${info.codec}` : ''}</p>
          <p>音轨：{info.audio.length ? info.audio.map(trackLabel).join('　') : '无'}</p>
          <p>字幕：{subs.length ? subs.map(s => s.label).join('　') : '无'}</p>
        {:else}
          <p>正在读取…</p>
        {/if}
        <p class="dim">切换音轨、音频延迟需要 mpv 播放引擎。</p>
      </div>
    {/if}
  </div>

  <div class="mp-osd" class:show={osdShow}>{osd}</div>

  <div class="mp-vhelp" role="presentation" onpointerdown={() => (helpOpen = false)}>
    <h4>视频快捷键与手势</h4>
    <div>
      <p><kbd>空格</kbd> / 单击画面　播放 / 暂停</p>
      <p><kbd>←</kbd> <kbd>→</kbd> 点按　后退 / 前进 5 秒</p>
      <p><kbd>→</kbd> 长按　3 倍速播放，松开恢复</p>
      <p><kbd>←</kbd> 长按　快速倒退，松开恢复</p>
      <p>鼠标左键长按画面　2 倍速，松开恢复</p>
      <p><kbd>↑</kbd> <kbd>↓</kbd> / 滚轮　音量 ±5%</p>
      <p><kbd>M</kbd>　静音</p>
      <p><kbd>[</kbd> <kbd>]</kbd>　减速 / 加速，<kbd>\</kbd> 恢复 1 倍</p>
      <p><kbd>,</kbd> <kbd>.</kbd>　上一帧 / 下一帧</p>
      <p><kbd>0</kbd>–<kbd>9</kbd>　跳到 0%–90%</p>
      <p><kbd>J</kbd> <kbd>L</kbd>　后退 / 前进 10 秒</p>
      <p>双击画面左 / 右三分之一　后退 / 前进 10 秒</p>
      <p><kbd>PgUp</kbd> <kbd>PgDn</kbd>　上一集 / 下一集</p>
      <p><kbd>Ctrl</kbd>+<kbd>→</kbd>　跳过片头（85 秒）</p>
      <p><kbd>C</kbd>　切换字幕，<kbd>V</kbd> 显示 / 隐藏</p>
      <p><kbd>D</kbd>　弹幕开关（有弹幕的视频）</p>
      <p><kbd>Z</kbd> <kbd>X</kbd>　字幕提前 / 延后 0.1 秒</p>
      <p><kbd>Ctrl</kbd>+<kbd>+</kbd> <kbd>-</kbd>　字幕放大 / 缩小</p>
      <p><kbd>S</kbd>　截图，<kbd>Ctrl</kbd>+<kbd>S</kbd> 带字幕截图</p>
      <p><kbd>R</kbd>　A-B 循环（设起点、终点、取消）</p>
      <p><kbd>F</kbd> / <kbd>Enter</kbd> / 双击中间　全屏</p>
      <p><kbd>P</kbd>　画中画，<kbd>T</kbd> 窗口置顶</p>
      <p><kbd>I</kbd> / <kbd>Tab</kbd>　媒体信息</p>
      <p><kbd>Esc</kbd>　退出全屏或关闭</p>
      <p><kbd>?</kbd>　显示 / 隐藏本列表</p>
    </div>
  </div>
</div>

<style>
  .dmb {
    font-size: 13px;
    font-weight: 700;
    opacity: 0.55;
  }
  .dmb.on {
    opacity: 1;
    color: var(--mp-accent);
  }
  .mp-vid {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    background: #000;
  }
  .mp-over.idle {
    cursor: none;
  }
  .grow {
    flex: 1;
  }
  .ttl {
    font-size: 15px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .vclose,
  .vtxt {
    background: none;
    border: 0;
    color: #fff;
    font-size: 20px;
    padding: 2px 8px;
    cursor: pointer;
  }
  .vtxt {
    font-size: 15px;
  }
  .mp-vtop {
    min-width: 0;
  }
  .mp-vtop .mp-tag {
    white-space: nowrap;
  }
  .mp-vrow .tm {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .mp-vrow .vol {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .mp-vrow .vol input {
    width: 84px;
    accent-color: #7aa2ff;
  }
  .mp-vrow .sel {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    color: #ffffffb0;
  }
  .mp-vrow .sel select {
    max-width: 240px;
  }
  .mp-vtip {
    position: absolute;
    bottom: 14px;
    translate: -50% 0;
    background: rgba(0, 0, 0, 0.75);
    padding: 3px 8px;
    border-radius: 6px;
    font-size: 12px;
    white-space: nowrap;
    pointer-events: none;
    font-variant-numeric: tabular-nums;
  }
  .mp-verr {
    position: absolute;
    inset: 0;
    margin: auto;
    height: fit-content;
    max-width: 560px;
    padding: 28px;
    border-radius: 16px;
    background: rgba(12, 12, 18, 0.9);
    border: 1px solid #ffffff22;
    display: grid;
    gap: 10px;
    align-content: center;
    text-align: center;
    line-height: 1.6;
  }
  .mp-verr .t {
    font-size: 18px;
    font-weight: 700;
    margin: 0;
  }
  .mp-verr p {
    margin: 0;
  }
  .mp-verr .hint {
    font-size: 12.5px;
    color: #ffffffa0;
  }
  .mp-verr .acts {
    display: flex;
    gap: 10px;
    justify-content: center;
  }
  .mp-next,
  .mp-skip {
    position: absolute;
    right: 28px;
    bottom: 140px;
    border: 1px solid #ffffff55;
    background: rgba(0, 0, 0, 0.6);
    color: #fff;
    border-radius: 999px;
    padding: 8px 18px;
    font-size: 13px;
    cursor: pointer;
  }
  .mp-skip {
    right: auto;
    left: 28px;
  }
  .mp-next:hover,
  .mp-skip:hover {
    border-color: #7aa2ff;
  }
  .vinfo {
    position: absolute;
    left: 28px;
    top: 80px;
    max-width: 620px;
    padding: 16px 18px;
    border-radius: 14px;
    background: rgba(0, 0, 0, 0.78);
    font-size: 13px;
    line-height: 1.7;
    display: grid;
    gap: 2px;
  }
  .vinfo p {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .vinfo .dim {
    color: #ffffff80;
    font-size: 12px;
  }
  .mp-over button:disabled {
    opacity: 0.35;
    cursor: default;
  }
  @media (max-width: 760px) {
    .mp-vrow .sel,
    .mp-vrow .vol input {
      display: none;
    }
  }
</style>
