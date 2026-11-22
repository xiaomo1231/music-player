// 播放引擎信息：web（WebView2 自带解码）或 mpv（libmpv，格式最全）
import { invoke, inTauri, type AppInfo } from './api';

class Engine {
  mpv = $state(false);
  version = $state('');
  dataDir = $state('');
  exclusive = $state(false);
  replaygain = $state(false);

  async load() {
    if (!inTauri) return;
    const info = await invoke<AppInfo>('app_info');
    this.mpv = info.engine === 'mpv';
    this.version = info.version;
    this.dataDir = info.data_dir;
    const s = await invoke<{ exclusive?: boolean; replaygain?: boolean } | null>('get_setting', { key: 'audio' });
    this.exclusive = !!s?.exclusive;
    this.replaygain = !!s?.replaygain;
  }
  private async persist() {
    await invoke('set_setting', { key: 'audio', value: { exclusive: this.exclusive, replaygain: this.replaygain } });
  }
  async setExclusive(v: boolean) {
    this.exclusive = v;
    await this.persist();
    if (this.mpv) await invoke('mpv_set_option', { name: 'audio-exclusive', value: v ? 'yes' : 'no' }).catch(() => {});
  }
  async setReplaygain(v: boolean) {
    this.replaygain = v;
    await this.persist();
    if (this.mpv) await invoke('mpv_set_option', { name: 'replaygain', value: v ? 'track' : 'no' }).catch(() => {});
  }
}
export const engine = new Engine();
engine.load().catch(() => {});
