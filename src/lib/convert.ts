// 格式转换页面的类型、常量和小工具（后端命令见 src-tauri/src/convert.rs）
export type ConvertAfter = 'lib' | 'replace' | 'side' | 'dir';
export type ConvertStatus = 'queued' | 'running' | 'done' | 'failed' | 'canceled' | 'undone';

export interface PresetInfo {
  id: string;
  name: string;
  /** audio：只输出音频；video：输入必须有视频；both：音视频都可以 */
  kind: 'audio' | 'video' | 'both';
  /** 输出扩展名，auto 表示由输入的音频编码决定 */
  ext: string;
  note: string;
}

export interface HwProbe { name: string; ok: boolean }
export interface HwStatus { checked: boolean; h264: string; hevc: string; probes: HwProbe[] }

export interface ConvertSettingsView {
  out_dir: string | null;
  out_dir_audio: string;
  out_dir_video: string;
  concurrency: number;
  plugin_enabled: boolean;
  plugin_ack: boolean;
  plugin_dir: string;
  plugin_file: string | null;
}

export interface ConvertJob {
  id: number;
  name: string;
  input: string;
  /** 完成后：实际输出文件 */
  output: string | null;
  /** 排队 / 失败时：请求的输出目录 */
  out_dir: string | null;
  preset: string;
  preset_name: string;
  after: ConvertAfter;
  status: ConvertStatus;
  progress: number;
  error: string | null;
  backup: string | null;
  can_undo: boolean;
  created_at: number;
  finished_at: number | null;
}

export interface ConvertProgress {
  id: number;
  progress: number;
  speed: number | null;
  status: ConvertStatus;
  error: string | null;
  output: string | null;
}

export const AFTER_OPTIONS: { value: ConvertAfter; label: string; note: string }[] = [
  { value: 'lib', label: '加入媒体库，保留原文件', note: '新文件保存到输出目录并自动加入媒体库，原文件不动。' },
  {
    value: 'replace',
    label: '替换原文件',
    note: '新文件接替原文件在媒体库里的位置，播放次数、收藏、歌单、观看进度都保留。原文件先移到回收站，随时可以撤销。',
  },
  { value: 'side', label: '放在原文件旁边', note: '新文件和原文件放在同一个文件夹，两者都在库里，播放时可以选用哪一个版本。' },
  { value: 'dir', label: '只输出到指定目录', note: '只生成文件，不改动媒体库。' },
];

export const AUDIO_EXTS = [
  'mp3', 'flac', 'm4a', 'aac', 'alac', 'wav', 'ape', 'wv', 'ogg', 'oga', 'opus', 'wma', 'aif', 'aiff',
  'dsf', 'dff', 'tta', 'tak', 'ac3', 'dts', 'mka',
];
export const VIDEO_EXTS = [
  'mp4', 'm4v', 'mkv', 'avi', 'mov', 'wmv', 'flv', 'webm', 'ts', 'm2ts', 'mts', 'rmvb', 'rm', '3gp', 'mpg', 'mpeg', 'vob', 'ogv',
];
/** 加密格式：需要解锁插件，文件对话框里可以选，但不能直接播放 */
export const ENCRYPTED_EXTS = ['qmc0', 'qmc3', 'qmcflac', 'qmcogg', 'mgg', 'mgg1', 'mflac', 'mflac0', 'kgm', 'vpr', 'ncm'];
export const MEDIA_EXTS = [...AUDIO_EXTS, ...VIDEO_EXTS, ...ENCRYPTED_EXTS];

export const STATUS_LABEL: Record<ConvertStatus, string> = {
  queued: '等待中',
  running: '转换中',
  done: '完成',
  failed: '失败',
  canceled: '已取消',
  undone: '已撤销',
};

export function extOf(p: string): string {
  const m = /\.([^./\\]+)$/.exec(p);
  return m ? m[1].toLowerCase() : '';
}

export const isAudioPath = (p: string) => AUDIO_EXTS.includes(extOf(p));
export const isVideoPath = (p: string) => VIDEO_EXTS.includes(extOf(p));

export function baseName(p: string): string {
  return p.split(/[\\/]/).pop() ?? p;
}

export function dirName(p: string): string {
  const i = Math.max(p.lastIndexOf('\\'), p.lastIndexOf('/'));
  return i > 0 ? p.slice(0, i) : p;
}

/** 编码器名 → 界面文字 */
export function encoderLabel(enc: string): string {
  if (enc.endsWith('_nvenc')) return 'NVIDIA NVENC';
  if (enc.endsWith('_qsv')) return 'Intel QSV';
  if (enc.endsWith('_amf')) return 'AMD AMF';
  if (enc === 'libx265') return '软件 x265';
  return '软件 x264';
}

const AFTER_KEY = 'mp.convert.after';

/** 记住上次选的“完成后”方式（只是界面偏好，存在本机） */
export function loadAfter(): ConvertAfter {
  try {
    const v = localStorage.getItem(AFTER_KEY);
    if (v === 'lib' || v === 'replace' || v === 'side' || v === 'dir') return v;
  } catch {
    // 无法访问本地存储时使用默认值
  }
  return 'lib';
}

export function saveAfter(v: ConvertAfter): void {
  try {
    localStorage.setItem(AFTER_KEY, v);
  } catch {
    // 忽略：不影响转换
  }
}

// ---------- 插件转换器 ----------

/** 插件转换任务的 preset：plg:<插件>/<转换器>:<输出扩展名或 same> */
export function converterPreset(plugin: string, id: string, ext: string): string {
  return `plg:${plugin}/${id}:${ext}`;
}

export function parseConverterPreset(preset: string): { plugin: string; id: string; ext: string } | null {
  const m = /^plg:([^/]+)\/(.+):([^:]+)$/.exec(preset);
  return m ? { plugin: m[1], id: m[2], ext: m[3] } : null;
}

/** 转换器能不能处理这个文件（和后端 accepts_ext 同一套规则） */
export function converterAccepts(inputs: string[], path: string): boolean {
  const e = extOf(path);
  return inputs.some(
    i => i === '*' || (i === 'audio' && AUDIO_EXTS.includes(e)) || (i === 'video' && VIDEO_EXTS.includes(e)) || (e !== '' && i === e),
  );
}
