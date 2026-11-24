<script lang="ts">
  import Icon from '../lib/Icon.svelte';
  import Cover from './Cover.svelte';
  import Slider from './Slider.svelte';
  import { player, PLAY_MODE_LABEL, PLAY_MODE_ICON } from '../lib/player.svelte';
  import { ui } from '../lib/ui.svelte';
  import { inTauri } from '../lib/api';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  const c = $derived(player.current);
  const k = $derived(player.lyricIndex());
  const cfg = $derived(ui.miniCfg);
  const line = $derived(cfg.line === 'lyric' && k >= 0 ? player.lyrics[k]?.text : (c?.artist ?? '♪'));
  function drag(e: MouseEvent) {
    if ((e.target as HTMLElement).closest('button,[role=slider]')) return;
    if (inTauri) getCurrentWindow().startDragging();
  }
  /** 退出迷你模式并打开迷你播放器设置 */
  async function settings() {
    await ui.setMini(false);
    ui.go('settings');
    ui.open('mini');
  }
</script>
<div class="mp-mini" data-bg={cfg.bg} class:nospin={!cfg.spin} style:--mp-accent={cfg.accent || null} role="presentation" onmousedown={drag}>
  <Cover cls="mp-mc" cover={c?.cover} name={c?.album ?? ''} hue={c?.hue} tag="div" />
  <div class="mp-mt">{c?.title ?? '未在播放'}</div>
  <div class="mp-ml">{line}</div>
  <div class="mp-mb">
    <button title={PLAY_MODE_LABEL[player.mode] + '（点击切换）'} aria-label={'播放模式：' + PLAY_MODE_LABEL[player.mode]} onclick={() => player.cycleMode()}><Icon name={PLAY_MODE_ICON[player.mode]} /></button>
    <button title="上一首" onclick={() => player.prev()}><Icon name="prev" /></button>
    <button title={player.playing ? '暂停' : '播放'} onclick={() => player.toggle()}><Icon name={player.playing ? 'pause' : 'play'} /></button>
    <button title="下一首" onclick={() => player.next()}><Icon name="next" /></button>
    <button title="迷你播放器设置" onclick={settings}><Icon name="gear" /></button>
    <button title="还原" onclick={() => ui.setMini(false)}><Icon name="expand" /></button>
  </div>
  <Slider cls="" mp="progress" value={player.dur ? player.t / player.dur : 0} onchange={v => player.seek(v * player.dur)} />
</div>
