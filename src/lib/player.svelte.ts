// 音乐播放器状态与引擎。
// 默认用 WebView2 自带的解码（HTMLAudioElement + Web Audio，支持 MP3/FLAC/WAV/OGG/Opus/AAC/M4A），
// 10 段均衡器和频谱都在 Web Audio 里实现；装了 libmpv 后 engine 换成 mpv，覆盖 APE/DSD/WMA 等全部格式。
import { invoke, listen, fileUrl, inTauri, type Track, type LyricLine, type OnlineLyrics } from './api';
import { engine } from './engine.svelte';
import { clamp, shuffleArr } from './util';
import { toast } from './ui.svelte';

export const EQF = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
export const EQ_PRESETS: Record<string, number[]> = {
  平直: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  流行: [-1, 1, 3, 4, 2, 0, -1, -1, 1, 2],
  摇滚: [5, 4, 2, 0, -1, -1, 1, 3, 4, 5],
  古典: [3, 2, 1, 0, 0, 0, -1, 1, 2, 3],
  人声: [-3, -2, 0, 2, 4, 4, 3, 1, 0, -1],
  低音增强: [7, 6, 4, 2, 0, 0, 0, 0, 0, 0],
  电子: [5, 4, 1, 0, -2, 1, 1, 2, 4, 5],
};
/** WebView2 能直接解码的格式；其余格式需要 mpv 引擎 */
const WEB_OK = new Set(['mp3', 'flac', 'wav', 'ogg', 'oga', 'opus', 'm4a', 'aac', 'webm', 'mp4']);

const load = <T>(k: string, d: T): T => {
  try {
    const v = localStorage.getItem('mp.' + k);
    return v == null ? d : JSON.parse(v);
  } catch {
    return d;
  }
};
const save = (k: string, v: unknown) => {
  try {
    localStorage.setItem('mp.' + k, JSON.stringify(v));
  } catch {}
};

class Player {
  queue = $state<Track[]>([]);
  order = $state<number[]>([]); // 随机模式下的播放顺序（queue 的下标）
  pos = $state(-1); // 在 order 里的位置
  playing = $state(false);
  t = $state(0);
  dur = $state(0);
  vol = $state(load('vol', 0.8));
  muted = $state(false);
  speed = $state(1);
  shuffle = $state(load('shuffle', false));
  repeat = $state<'off' | 'all' | 'one'>(load('repeat', 'all'));
  lyrics = $state<LyricLine[]>([]);
  eq = $state<number[]>(load('eq', [0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
  error = $state('');
  source = 'library';
  /** 当前曲目用哪个引擎：web（WebView2）或 mpv */
  backend = $state<'web' | 'mpv'>('web');

  el: HTMLAudioElement | null = null;
  ctx: AudioContext | null = null;
  analyser: AnalyserNode | null = null;
  filters: BiquadFilterNode[] = [];
  gain: GainNode | null = null;
  private listened = 0;
  private lastTick = 0;
  private fbuf: Uint8Array<ArrayBuffer> | null = null;

  get current(): Track | null {
    const i = this.order[this.pos];
    return i == null ? null : this.queue[i] ?? null;
  }

  private ensure() {
    if (this.el) return;
    const el = new Audio();
    el.crossOrigin = 'anonymous';
    el.preload = 'auto';
    this.el = el;
    el.addEventListener('loadedmetadata', () => (this.dur = el.duration || this.current?.duration || 0));
    el.addEventListener('ended', () => this.onEnded());
    el.addEventListener('error', () => {
      if (this.backend === 'mpv' || !el.getAttribute('src')) return;
      const c = this.current;
      this.error = c ? `无法播放：${c.title}` : '无法播放';
      toast(this.error + (c && !WEB_OK.has(ext(c.path)) ? `（${c.format} 需要 mpv 播放引擎）` : ''));
      this.playing = false;
    });
    el.addEventListener('timeupdate', () => this.backend === 'web' && (this.t = el.currentTime));
    try {
      const ctx = new AudioContext();
      const src = ctx.createMediaElementSource(el);
      this.filters = EQF.map((f, i) => {
        const b = ctx.createBiquadFilter();
        b.type = i === 0 ? 'lowshelf' : i === EQF.length - 1 ? 'highshelf' : 'peaking';
        b.frequency.value = f;
        b.Q.value = 1.1;
        b.gain.value = this.eq[i] ?? 0;
        return b;
      });
      this.gain = ctx.createGain();
      this.analyser = ctx.createAnalyser();
      this.analyser.fftSize = 2048;
      this.analyser.smoothingTimeConstant = 0.78;
      let node: AudioNode = src;
      for (const f of this.filters) {
        node.connect(f);
        node = f;
      }
      node.connect(this.gain);
      this.gain.connect(this.analyser);
      this.analyser.connect(ctx.destination);
      this.ctx = ctx;
      this.fbuf = new Uint8Array(new ArrayBuffer(this.analyser.frequencyBinCount));
    } catch (e) {
      console.warn('Web Audio 不可用，均衡器和频谱停用', e);
    }
    this.applyVol();
    this.ticker = setInterval(this.loop, 100);
  }

  private ticker: ReturnType<typeof setInterval> | undefined;
  // 播放进度和收听时长：10 次 / 秒就够了，不需要跟着屏幕刷新率（175 Hz）跑。暂停时只做一次空判断
  private loop = () => {
    const now = performance.now();
    if (this.el && this.playing) {
      if (this.backend === 'web') this.t = this.el.currentTime;
      if (this.lastTick) this.listened += Math.min(0.25, (now - this.lastTick) / 1000) * this.speed;
    }
    this.lastTick = now;
  };

  /** 替换队列并从第 i 首开始 */
  playList(list: Track[], i = 0, source = 'library') {
    if (!list.length) return;
    this.flushStats();
    this.source = source;
    this.queue = list;
    this.buildOrder(i);
    this.load(true);
  }

  playOne(t: Track) {
    const i = this.queue.findIndex(x => x.id === t.id);
    if (i >= 0) {
      this.flushStats();
      this.pos = this.order.indexOf(i);
      this.load(true);
    } else this.playList([t], 0);
  }

  /** 下一首播放 */
  playNext(tracks: Track[]) {
    if (!this.current) return this.playList(tracks);
    const have = new Set(this.queue.map(q => q.id));
    const add = tracks.filter(t => !have.has(t.id));
    const base = this.queue.length;
    this.queue = [...this.queue, ...add];
    const idxs = add.map((_, k) => base + k);
    const o = [...this.order];
    o.splice(this.pos + 1, 0, ...idxs);
    this.order = o;
    toast(`已添加 ${add.length} 首到下一首播放`);
  }

  enqueue(tracks: Track[]) {
    if (!this.current) return this.playList(tracks);
    const have = new Set(this.queue.map(q => q.id));
    const add = tracks.filter(t => !have.has(t.id));
    const base = this.queue.length;
    this.queue = [...this.queue, ...add];
    this.order = [...this.order, ...add.map((_, k) => base + k)];
    toast(`已加入播放队列：${add.length} 首`);
  }

  removeAt(orderPos: number) {
    if (orderPos === this.pos) return;
    const o = [...this.order];
    o.splice(orderPos, 1);
    if (orderPos < this.pos) this.pos--;
    this.order = o;
  }

  private buildOrder(start: number) {
    const idx = this.queue.map((_, i) => i);
    if (this.shuffle) {
      const rest = shuffleArr(idx.filter(i => i !== start));
      this.order = [start, ...rest];
      this.pos = 0;
    } else {
      this.order = idx;
      this.pos = start;
    }
  }

  private async load(autoplay: boolean) {
    const c = this.current;
    if (!c) return;
    this.ensure();
    this.error = '';
    this.t = 0;
    this.dur = c.duration;
    this.listened = 0;
    this.lyrics = [];
    document.documentElement.style.setProperty('--hue', String(c.hue ?? hueFromName(c.album || c.title)));
    if (c.encrypted) {
      toast('加密格式需要先安装解锁插件，并在“格式转换”里转成普通格式');
      this.playing = false;
      return;
    }
    this.backend = engine.mpv && (!WEB_OK.has(ext(c.path)) || engine.exclusive || engine.replaygain) ? 'mpv' : 'web';
    if (this.backend === 'mpv') {
      this.el!.pause();
      this.el!.removeAttribute('src');
      this.el!.load();
      try {
        await invoke('mpv_eq', { gains: this.eq });
        await invoke('mpv_volume', { volume: this.muted ? 0 : this.vol });
        await invoke('mpv_speed', { speed: this.speed });
        await invoke('mpv_load', { path: c.path, paused: !autoplay });
        this.playing = autoplay;
      } catch (e) {
        toast('mpv 播放失败：' + e);
        this.playing = false;
      }
    } else {
      if (engine.mpv) invoke('mpv_stop').catch(() => {});
      this.el!.src = fileUrl(c.path);
      this.el!.playbackRate = this.speed;
      if (autoplay) this.play();
    }
    if (inTauri) {
      invoke<LyricLine[]>('lyrics', { path: c.path })
        .then(async l => {
          if (this.current?.id !== c.id) return;
          this.lyrics = l;
          // 本地没有歌词时在线查找（设置里可关闭，后端会直接返回空）
          if (l.length) return;
          const o = await invoke<OnlineLyrics | null>('lyrics_online', {
            title: c.title,
            artist: c.artist,
            album: c.album,
            duration: c.duration,
          });
          if (o && this.current?.id === c.id && !this.lyrics.length) this.lyrics = o.lines;
        })
        .catch(() => {});
    }
    updateMediaSession(c);
  }

  play() {
    if (!this.el) {
      if (this.current) this.load(true);
      return;
    }
    if (this.backend === 'mpv') {
      invoke('mpv_pause', { paused: false }).catch(() => {});
      this.playing = true;
      return;
    }
    if (this.ctx?.state === 'suspended') this.ctx.resume();
    this.el.play().then(
      () => (this.playing = true),
      e => {
        if (e?.name !== 'AbortError') this.playing = false;
      },
    );
    this.playing = true;
  }

  pause() {
    if (this.backend === 'mpv') invoke('mpv_pause', { paused: true }).catch(() => {});
    this.el?.pause();
    this.playing = false;
  }

  toggle() {
    if (!this.current) return;
    this.playing ? this.pause() : this.play();
  }

  next(auto = false) {
    if (!this.order.length) return;
    this.flushStats();
    if (this.pos + 1 >= this.order.length) {
      if (this.repeat === 'off' && auto) {
        this.pause();
        this.seek(0);
        return;
      }
      if (this.shuffle) this.order = shuffleArr(this.order);
      this.pos = 0;
    } else this.pos++;
    this.load(true);
  }

  prev() {
    if (this.t > 4) return this.seek(0);
    if (!this.order.length) return;
    this.flushStats();
    this.pos = (this.pos - 1 + this.order.length) % this.order.length;
    this.load(true);
  }

  jump(orderPos: number) {
    this.flushStats();
    this.pos = orderPos;
    this.load(true);
  }

  /** mpv 引擎的状态事件 */
  onMpvState(s: { time: number; duration: number; paused: boolean }) {
    if (this.backend !== 'mpv') return;
    this.t = s.time;
    if (s.duration) this.dur = s.duration;
  }
  onMpvEnd(natural: boolean) {
    if (this.backend === 'mpv' && natural) this.onEnded();
  }

  private onEnded() {
    if (this.repeat === 'one') {
      this.flushStats();
      this.seek(0);
      this.play();
    } else this.next(true);
  }

  seek(t: number) {
    const d = this.dur || 0;
    this.t = clamp(t, 0, Math.max(0, d - 0.2));
    if (this.backend === 'mpv') invoke('mpv_seek', { time: this.t }).catch(() => {});
    else if (this.el) this.el.currentTime = this.t;
  }

  setVol(v: number) {
    this.vol = clamp(v);
    this.muted = false;
    save('vol', this.vol);
    this.applyVol();
  }

  toggleMute() {
    this.muted = !this.muted;
    this.applyVol();
  }

  private applyVol() {
    const v = this.muted ? 0 : this.vol * this.vol; // 平方让音量条听感更线性
    if (engine.mpv) invoke('mpv_volume', { volume: Math.sqrt(v) }).catch(() => {});
    if (this.gain) {
      this.el!.volume = 1;
      this.gain.gain.value = v;
    } else if (this.el) this.el.volume = v;
  }

  setSpeed(v: number) {
    this.speed = v;
    if (this.backend === 'mpv') invoke('mpv_speed', { speed: v }).catch(() => {});
    if (this.el) {
      this.el.playbackRate = v;
      this.el.preservesPitch = true;
    }
  }

  setEq(gains: number[]) {
    this.eq = gains;
    gains.forEach((g, i) => this.filters[i] && (this.filters[i].gain.value = g));
    if (this.backend === 'mpv') invoke('mpv_eq', { gains }).catch(() => {});
    save('eq', gains);
  }

  toggleShuffle() {
    this.shuffle = !this.shuffle;
    save('shuffle', this.shuffle);
    const cur = this.order[this.pos];
    if (cur != null) this.buildOrder(cur);
    toast(this.shuffle ? '随机播放' : '顺序播放');
  }

  cycleRepeat() {
    this.repeat = this.repeat === 'all' ? 'one' : this.repeat === 'one' ? 'off' : 'all';
    save('repeat', this.repeat);
    toast({ all: '列表循环', one: '单曲循环', off: '不循环' }[this.repeat]);
  }

  /** 切歌或播完时把这次收听写进播放记录 */
  flushStats() {
    const c = this.current;
    if (!c || !inTauri || this.listened < 1) return;
    invoke('record_play', { id: c.id, listened: this.listened, duration: this.dur || c.duration, source: this.source }).catch(() => {});
    this.listened = 0;
  }

  /** 频谱：n 段，0–1 */
  private sm: number[] = [];
  private ph = 0;
  spectrum(n = 48): number[] {
    const out = new Array<number>(n);
    if (this.analyser && this.fbuf && this.playing && this.ctx?.state === 'running') {
      this.analyser.getByteFrequencyData(this.fbuf);
      const N = this.fbuf.length,
        lo = 2,
        hi = N * 0.62;
      for (let i = 0; i < n; i++) {
        const a = lo * Math.pow(hi / lo, i / n),
          b = lo * Math.pow(hi / lo, (i + 1) / n);
        let m = 0,
          k = 0;
        for (let j = a | 0; j <= (b | 0) && j < N; j++) {
          m += this.fbuf[j];
          k++;
        }
        out[i] = clamp(Math.pow(m / Math.max(k, 1) / 255, 1.25) * (1 + (i / n) * 1.1));
      }
    } else {
      this.ph += 0.016;
      const pl = this.playing ? 0.35 : 0.1;
      for (let i = 0; i < n; i++) {
        const x = i / n;
        out[i] = clamp(pl * (0.42 * (1 - x * 0.6) * (0.55 + 0.45 * Math.sin(this.ph * 3.1 + i * 0.9) * Math.sin(this.ph * 1.7 + i * 0.31))) + 0.02);
      }
    }
    for (let i = 0; i < n; i++) {
      const s = this.sm[i];
      this.sm[i] = s == null ? out[i] : out[i] > s ? out[i] : s * 0.9 + out[i] * 0.1;
      out[i] = this.sm[i];
    }
    return out;
  }

  // ---------------------------------------------------------------- 动效用的音频数据
  // 和 spectrum() 分开保存状态：背景、唱片环、设置预览同一帧里各自调用也不会互相干扰。
  private fxCache = new Map<number, { at: number; v: number[] }>();
  private fxLv = { bass: 0, level: 0, beat: 0, at: 0 };
  private fxBassAvg = 0.2;
  private fxBeatAt = 0;

  /** 是否有真实的音频数据（WebView2 引擎播放中）；mpv 引擎没有 Web Audio 数据，动效改用模拟律动 */
  fxLive(): boolean {
    return this.backend === 'web' && !!this.analyser && !!this.fbuf && this.playing && this.ctx?.state === 'running';
  }

  /** 动效用频谱：n 段，0–1，低频在前；同一帧重复调用直接返回缓存 */
  fxBands(n = 32): number[] {
    const now = performance.now();
    const c = this.fxCache.get(n);
    if (c && now - c.at < 8) return c.v;
    const out = new Array<number>(n);
    if (this.fxLive()) {
      this.analyser!.getByteFrequencyData(this.fbuf!);
      const f = this.fbuf!,
        N = f.length,
        lo = 2,
        hi = N * 0.62;
      for (let i = 0; i < n; i++) {
        const a = lo * Math.pow(hi / lo, i / n),
          b = lo * Math.pow(hi / lo, (i + 1) / n);
        let m = 0,
          k = 0;
        for (let j = a | 0; j <= (b | 0) && j < N; j++) {
          m += f[j];
          k++;
        }
        out[i] = clamp(Math.pow(m / Math.max(k, 1) / 255, 1.4) * (1 + (i / n) * 1.3));
      }
    } else {
      // 模拟律动：按时间生成（不按调用次数），播放时有起伏，暂停时回落到很低的呼吸
      const t = now / 1000,
        pl = this.playing ? 1 : 0.12,
        kick = this.playing ? Math.exp(-((t * 1.9) % 1) * 5) : 0;
      for (let i = 0; i < n; i++) {
        const x = i / n;
        const wob = 0.5 + 0.5 * Math.sin(t * 2.3 + i * 0.7) * Math.sin(t * 1.3 + i * 0.23 + 1.1);
        const body = (0.62 - x * 0.38) * (0.35 + 0.65 * wob);
        out[i] = clamp(pl * (body + kick * 0.45 * Math.max(0, 1 - x * 2.2)) + 0.015);
      }
    }
    const prev = c?.v;
    if (prev && prev.length === n) {
      // 上升快、回落慢
      for (let i = 0; i < n; i++) out[i] = out[i] > prev[i] ? prev[i] + (out[i] - prev[i]) * 0.6 : prev[i] * 0.86 + out[i] * 0.14;
    }
    this.fxCache.set(n, { at: now, v: out });
    return out;
  }

  /** 动效用响度：bass 低频、level 整体响度、beat 节拍脉冲（触发时为 1，随后衰减），都在 0–1 */
  levels(): { bass: number; level: number; beat: number } {
    const now = performance.now();
    const L = this.fxLv;
    if (now - L.at < 8) return L;
    const dt = Math.min(0.1, L.at ? (now - L.at) / 1000 : 0.016);
    L.at = now;
    let bass = 0,
      level = 0;
    if (this.fxLive()) {
      this.analyser!.getByteFrequencyData(this.fbuf!);
      const f = this.fbuf!,
        hz = (this.ctx!.sampleRate || 48000) / (this.analyser!.fftSize || 2048);
      const b1 = Math.max(1, Math.round(40 / hz)),
        b2 = Math.max(b1 + 1, Math.round(160 / hz)),
        top = Math.min(f.length, Math.round(12000 / hz));
      let s = 0;
      for (let j = b1; j <= b2; j++) s += f[j];
      bass = clamp(Math.pow(s / (b2 - b1 + 1) / 255, 1.6) * 1.25);
      s = 0;
      for (let j = 1; j < top; j++) s += f[j];
      level = clamp(Math.pow(s / Math.max(1, top - 1) / 255, 0.9) * 1.6);
    } else if (this.playing) {
      const t = now / 1000,
        kick = Math.exp(-((t * 1.9) % 1) * 5);
      bass = clamp(0.25 + 0.55 * kick + 0.08 * Math.sin(t * 0.7));
      level = clamp(0.45 + 0.15 * Math.sin(t * 0.37) + 0.1 * kick);
    }
    // 节拍：低频明显高于最近的平均值
    this.fxBassAvg += (bass - this.fxBassAvg) * Math.min(1, dt * 1.5);
    let beat = L.beat * Math.exp(-dt * 5);
    if (bass > 0.2 && bass > this.fxBassAvg * 1.3 && now - this.fxBeatAt > 280) {
      beat = 1;
      this.fxBeatAt = now;
    }
    const ease = (a: number, b: number) => (b > a ? a + (b - a) * Math.min(1, dt * 18) : a + (b - a) * Math.min(1, dt * 4));
    L.bass = ease(L.bass, bass);
    L.level = ease(L.level, level);
    L.beat = beat;
    return L;
  }

  /** 当前歌词行 */
  lyricIndex(): number {
    const l = this.lyrics;
    if (!l.length || l[0].t < 0) return -1;
    let k = -1;
    for (let i = 0; i < l.length; i++) {
      if (l[i].t <= this.t + 0.15) k = i;
      else break;
    }
    return k;
  }

  /** 当前行唱到哪里（0–1），逐字歌词按字计算 */
  lyricProgress(k: number): number {
    const l = this.lyrics;
    if (k < 0 || !l[k]) return 0;
    const line = l[k];
    if (line.words.length) {
      const total = line.words.reduce((s, w) => s + w[1].length, 0);
      let done = 0;
      for (let i = 0; i < line.words.length; i++) {
        const [st, w] = line.words[i];
        const en = line.words[i + 1]?.[0] ?? l[k + 1]?.t ?? st + 1;
        if (this.t >= en) done += w.length;
        else if (this.t >= st) done += w.length * clamp((this.t - st) / Math.max(0.05, en - st));
      }
      return clamp(done / Math.max(1, total));
    }
    const a = line.t,
      b = l[k + 1]?.t ?? this.dur;
    return clamp((this.t - a) / Math.min(Math.max(0.1, b - a), 6));
  }
}

const ext = (p: string) => p.split('.').pop()?.toLowerCase() ?? '';
const hueFromName = (s: string) => {
  let h = 0;
  for (const c of s) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
};

function updateMediaSession(c: Track) {
  if (!('mediaSession' in navigator)) return;
  navigator.mediaSession.metadata = new MediaMetadata({ title: c.title, artist: c.artist, album: c.album });
}

export const player = new Player();

if (typeof navigator !== 'undefined' && 'mediaSession' in navigator) {
  // 接入 Windows 系统媒体控制（音量浮层、锁屏、耳机按键）
  const ms = navigator.mediaSession;
  ms.setActionHandler('play', () => player.play());
  ms.setActionHandler('pause', () => player.pause());
  ms.setActionHandler('nexttrack', () => player.next());
  ms.setActionHandler('previoustrack', () => player.prev());
  ms.setActionHandler('seekto', d => d.seekTime != null && player.seek(d.seekTime));
}
if (typeof window !== 'undefined') window.addEventListener('beforeunload', () => player.flushStats());
listen<{ time: number; duration: number; paused: boolean }>('mpv://state', s => player.onMpvState(s));
listen<boolean>('mpv://end', n => player.onMpvEnd(n));
listen<string>('mpv://error', m => toast('mpv：' + m));
