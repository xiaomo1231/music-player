<script lang="ts">
  import Icon from '../lib/Icon.svelte';
  import Cover from './Cover.svelte';
  import Slider from './Slider.svelte';
  import { player } from '../lib/player.svelte';
  import { ui } from '../lib/ui.svelte';
  import { inTauri } from '../lib/api';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  const c = $derived(player.current);
  const k = $derived(player.lyricIndex());
  function drag(e: MouseEvent) {
    if ((e.target as HTMLElement).closest('button,[role=slider]')) return;
    if (inTauri) getCurrentWindow().startDragging();
  }
</script>
<div class="mp-mini" role="presentation" onmousedown={drag}>
  <Cover cls="mp-mc" cover={c?.cover} name={c?.album ?? ''} hue={c?.hue} tag="div" />
  <div class="mp-mt">{c?.title ?? '未在播放'}</div>
  <div class="mp-ml">{k >= 0 ? player.lyrics[k]?.text : (c?.artist ?? '♪')}</div>
  <div class="mp-mb">
    <button onclick={() => player.prev()}><Icon name="prev" /></button>
    <button onclick={() => player.toggle()}><Icon name={player.playing ? 'pause' : 'play'} /></button>
    <button onclick={() => player.next()}><Icon name="next" /></button>
    <button title="还原" onclick={() => ui.setMini(false)}><Icon name="expand" /></button>
  </div>
  <Slider cls="" mp="progress" value={player.dur ? player.t / player.dur : 0} onchange={v => player.seek(v * player.dur)} />
</div>
