// 检查更新：从 GitHub Releases 读取 latest.json，有新版本时弹窗让用户选择是否更新。
// 用户点“立即更新”才下载安装包，校验签名后覆盖安装并重启。
import { check, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { inTauri } from './api';
import { getSetting, setSetting } from './shell.svelte';
import { toast } from './ui.svelte';

type Phase = 'idle' | 'checking' | 'available' | 'downloading' | 'installing' | 'error';

class Updater {
  phase = $state<Phase>('idle');
  update = $state<Update | null>(null);
  /** 弹窗是否打开 */
  show = $state(false);
  downloaded = $state(0);
  total = $state(0);
  error = $state('');
  lastChecked = $state<number | null>(null);

  get version() {
    return this.update?.version ?? '';
  }
  get notes() {
    return this.update?.body?.trim() ?? '';
  }
  get busy() {
    return this.phase === 'checking' || this.phase === 'downloading' || this.phase === 'installing';
  }

  /**
   * 检查更新。manual = true 时（设置页按钮）无论结果都给提示；
   * 启动时的自动检查只在有新版本、且用户没选过“跳过这个版本”时弹窗，出错不打扰。
   */
  async check(manual = false) {
    if (!inTauri || this.busy) return;
    this.phase = 'checking';
    this.error = '';
    try {
      const u = await check({ timeout: 15000 });
      this.lastChecked = Date.now();
      if (!u) {
        this.update = null;
        this.phase = 'idle';
        if (manual) toast('已经是最新版本');
        return;
      }
      this.update = u;
      this.phase = 'available';
      if (!manual && (await getSetting<string>('update_skip', '')) === u.version) return;
      this.show = true;
    } catch (e) {
      this.phase = 'error';
      this.error = String(e);
      if (manual) toast('检查更新失败：' + this.error);
    }
  }

  /** 启动后延迟几秒检查一次（设置里可以关掉） */
  async autoCheck() {
    if (!inTauri || import.meta.env.DEV) return;
    if (!(await getSetting('update_auto_check', true))) return;
    setTimeout(() => this.check(false), 5000);
  }

  async install() {
    const u = this.update;
    if (!u || this.busy) return;
    this.phase = 'downloading';
    this.downloaded = 0;
    this.total = 0;
    try {
      await u.downloadAndInstall(ev => {
        if (ev.event === 'Started') this.total = ev.data.contentLength ?? 0;
        else if (ev.event === 'Progress') this.downloaded += ev.data.chunkLength;
        else if (ev.event === 'Finished') this.phase = 'installing';
      });
      // Windows 上安装程序会自己结束本程序；走到这里说明需要手动重启
      await relaunch();
    } catch (e) {
      this.phase = 'error';
      this.error = String(e);
    }
  }

  async skip() {
    if (this.update) await setSetting('update_skip', this.update.version);
    this.later();
  }

  later() {
    if (this.phase === 'downloading' || this.phase === 'installing') return;
    this.show = false;
    if (this.phase === 'error') this.phase = this.update ? 'available' : 'idle';
  }
}

export const updater = new Updater();
