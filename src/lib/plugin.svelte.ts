// 插件：类型、命令封装、安装确认队列、错误码文案。格式和协议见 docs/插件开发.md
import { invoke, inTauri, listen } from './api';
import { toast } from './ui.svelte';

export interface PluginStatus {
  method: string;
  /** 空表示上次成功 */
  code: string;
  message: string;
  at: number;
}

export interface PluginSourceView {
  id: string;
  provider_id: string;
  name: string;
  media_kinds: string[];
  capabilities: string[];
  status: PluginStatus | null;
}

export interface ConverterOutput {
  /** 扩展名；same 表示和输入相同 */
  ext: string;
  name: string;
}

/** 插件提供的格式转换器（plugin.json 的 contributes.converters） */
export interface ConverterView {
  plugin: string;
  plugin_name: string;
  id: string;
  name: string;
  /** 扩展名，或 audio / video / * */
  inputs: string[];
  outputs: ConverterOutput[];
  options: SettingDef[];
  note: string;
  /** 直接运行命令行程序（没有进度） */
  exec: boolean;
  needs_settings: string[];
}

export interface PluginUpdate {
  version: string;
  repo: string;
  /** 新版本多要的权限；空表示可以直接更新 */
  new_permissions: string[];
}

export interface PluginView {
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  homepage: string;
  license: string;
  /** rule / process / js */
  runtime: string;
  /** limited（规则、js 沙箱）/ full（程序） */
  trust: string;
  enabled: boolean;
  dev: boolean;
  dir: string;
  problem: string;
  hosts: string[];
  login: 'none' | 'optional' | 'required';
  login_granted: boolean;
  tools: string[];
  icon: string;
  readme: string;
  sources: PluginSourceView[];
  converters: ConverterView[];
  needs_settings: string[];
  has_settings: boolean;
  rules: string[];
  origin: string;
  installed_at: number;
  updated_at: number;
  has_prev: boolean;
  sha256: string;
  update: PluginUpdate | null;
}

export interface SettingDef {
  key: string;
  /** string / secret / int / bool / select / folder / file */
  type: string;
  label: string;
  help: string;
  default: unknown;
  required: boolean;
  options: (string | { value: string; label?: string })[];
}

export interface PluginPreview {
  ok: boolean;
  issues: string[];
  id: string;
  name: string;
  version: string;
  author: string;
  description: string;
  runtime: string;
  trust: string;
  media_kinds: string[];
  sources: string[];
  /** 格式转换器：“名字（输入 → 输出）” */
  converters: string[];
  hosts: string[];
  lan_hosts: string[];
  login: 'none' | 'optional' | 'required';
  tools: string[];
  rules: string[];
  settings: SettingDef[];
  readme: string;
  installed_version: string | null;
  new_permissions: string[];
  downgrade: boolean;
  dev_installed: boolean;
}

export interface SettingsView {
  schema: SettingDef[];
  values: Record<string, unknown>;
  /** 已经保存过的密钥（不回传内容） */
  secrets_set: string[];
}

export interface LogLine {
  at: number;
  method: string;
  level: string;
  msg: string;
}

export interface Stage {
  name: string;
  ok: boolean;
  ms: number;
  error: string;
  code: string;
  summary: string;
  preview: unknown;
}

export interface RepoEntry {
  id: string;
  name: string;
  version: string;
  url: string;
  sha256: string;
  app: string;
  runtime: string;
  media_kinds: string[];
  hosts: string[];
  login: string;
  tools: string[];
  description?: string;
  author?: string;
  installed_version: string | null;
  /** install / update / installed / newer_app */
  state: string;
}

export interface RepoView {
  url: string;
  name: string;
  last_check: number;
  last_error: string;
  plugins: RepoEntry[];
}

export const RUNTIME_TEXT: Record<string, string> = { rule: '规则', process: '程序', js: '脚本（沙箱）' };
export const TOOL_TEXT: Record<string, string> = { ytdlp: 'yt-dlp', ffmpeg: 'ffmpeg', ffprobe: 'ffprobe' };

/** 错误码 → 给用户看的说明和建议操作 */
export const CODE_TEXT: Record<string, { label: string; hint: string; action?: 'login' | 'settings' | 'enable' | 'logs' }> = {
  login_required: { label: '需要登录', hint: '先在“网站登录”里自己登录这个网站，再回来重试。软件和插件都不会替你登录。', action: 'login' },
  captcha: { label: '需要验证', hint: '网站要求人机验证。请在浏览器或“网站登录”里自己完成后再试。', action: 'login' },
  forbidden: { label: '被拒绝', hint: '网站拒绝了访问，或插件访问了没有声明的网站。', action: 'logs' },
  not_found: { label: '没找到', hint: '网站上没有这个内容，或地址变了。' },
  rate_limited: { label: '太频繁', hint: '网站限制了请求频率，过一会儿再试。' },
  network: { label: '网络', hint: '网络连接失败或超时。', action: 'logs' },
  bad_config: { label: '设置不对', hint: '插件的设置没填或不正确。', action: 'settings' },
  unsupported: { label: '不支持', hint: '这个插件不支持这个操作。' },
  internal: { label: '插件出错', hint: '插件自己出错了，可以看日志或联系作者。', action: 'logs' },
  plugin_disabled: { label: '已停用', hint: '插件已停用，启用后可以继续。', action: 'enable' },
};

/** 从带错误码的消息里拆出错误码：`[code] 说明` */
export function splitCode(msg: string): { code: string; text: string } {
  const m = /^\[([a-z_]+)\] ([\s\S]*)$/.exec(msg);
  return m && CODE_TEXT[m[1]] ? { code: m[1], text: m[2] } : { code: '', text: msg };
}

export const isPluginPkg = (p: string) => /\.mpx$/i.test(p);

class PluginState {
  /** 等待确认安装的包（.mpx 文件或插件文件夹），一个一个确认 */
  pending = $state<string[]>([]);
  /** 从仓库安装：{repo, id} */
  pendingRepo = $state<{ repo: string; id: string } | null>(null);
  /** 打开详情的插件 */
  detail = $state('');
  /** 插件有变化时 +1 */
  version = $state(0);
}
export const plugins = new PluginState();

export const pluginApi = {
  list: () => (inTauri ? invoke<PluginView[]>('plugin_list') : Promise.resolve([] as PluginView[])),
  inspect: (path: string) => invoke<{ kind: 'plugin' | 'em' | 'exe'; preview: PluginPreview | null }>('plugin_inspect', { path }),
  install: (path: string, allowLogin: boolean) => invoke<PluginView>('plugin_install', { path, allowLogin }),
  loadDev: (path: string) => invoke<PluginView>('plugin_load_dev', { path }),
  setEnabled: (id: string, enabled: boolean) => invoke<PluginView>('plugin_set_enabled', { id, enabled }),
  setLogin: (id: string, allow: boolean) => invoke<PluginView>('plugin_set_login', { id, allow }),
  uninstall: (id: string, purge: boolean) => invoke<void>('plugin_uninstall', { id, purge }),
  rollback: (id: string) => invoke<PluginView>('plugin_rollback', { id }),
  logs: (id: string) => invoke<LogLine[]>('plugin_logs', { id }),
  clearLogs: (id: string) => invoke<void>('plugin_clear_logs', { id }),
  openDir: (id: string) => invoke<void>('plugin_open_dir', { id }),
  pack: (id: string, path: string) => invoke<void>('plugin_pack', { id, path }),
  packDir: (dir: string, path: string) => invoke<void>('plugin_pack_dir', { dir, path }),
  test: (id: string, source: string, query: string, mediaKind: string) => invoke<Stage[]>('plugin_test', { id, source, query, mediaKind }),
  getSettings: (id: string) => invoke<SettingsView>('plugin_get_settings', { id }),
  saveSettings: (id: string, values: Record<string, unknown>) => invoke<void>('plugin_save_settings', { id, values }),
  repos: () => invoke<RepoView[]>('plugin_repos'),
  repoAdd: (url: string) => invoke<RepoView[]>('plugin_repo_add', { url }),
  repoRemove: (url: string) => invoke<void>('plugin_repo_remove', { url }),
  repoRefresh: () => invoke<RepoView[]>('plugin_repo_refresh'),
  repoInspect: (repo: string, id: string) => invoke<PluginPreview>('plugin_repo_inspect', { repo, id }),
  repoInstall: (repo: string, id: string, allowLogin: boolean) => invoke<PluginView>('plugin_repo_install', { repo, id, allowLogin }),
  update: (id: string) => invoke<PluginView>('plugin_update', { id }),
  getAutoUpdate: () => invoke<boolean>('plugin_get_auto_update'),
  setAutoUpdate: (on: boolean) => invoke<void>('plugin_set_auto_update', { on }),
};

/** 打开、拖入的 .mpx：进入安装确认 */
export function installPlugins(paths: string[]) {
  const add = paths.filter(p => !plugins.pending.includes(p));
  if (add.length) plugins.pending = [...plugins.pending, ...add];
}

/** 把 .mpx 从一批路径里分出来；返回剩下的路径 */
export function takePlugins(paths: string[]): string[] {
  const pk = paths.filter(isPluginPkg);
  if (pk.length) installPlugins(pk);
  return paths.filter(p => !isPluginPkg(p));
}

/** “安装插件…”：选 .mpx 文件 */
export async function pickPlugin() {
  if (!inTauri) return toast('请在桌面程序里安装插件');
  const { open } = await import('@tauri-apps/plugin-dialog');
  const p = await open({ multiple: true, filters: [{ name: '插件包', extensions: ['mpx'] }] });
  if (!p) return;
  installPlugins(Array.isArray(p) ? p : [p]);
}

/** “从文件夹安装…”：选含 plugin.json 的文件夹 */
export async function pickPluginFolder() {
  if (!inTauri) return;
  const { open } = await import('@tauri-apps/plugin-dialog');
  const p = await open({ directory: true });
  if (typeof p === 'string' && p) installPlugins([p]);
}

let started = false;
/** 启动时调用一次：监听后端的插件变化 */
export function startPlugins() {
  if (started || !inTauri) return;
  started = true;
  void listen<string>('plugin://changed', () => {
    plugins.version++;
  });
}

export const fmtTime = (t: number) => (t ? new Date(t * 1000).toLocaleString() : '');
