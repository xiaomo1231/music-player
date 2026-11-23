<script lang="ts">
  import Icon from '../lib/Icon.svelte';
  import Slider from './Slider.svelte';
  import Cover from './Cover.svelte';
  import { player, PLAY_MODE_ICON, PLAY_MODE_LABEL } from '../lib/player.svelte';
  import { ui } from '../lib/ui.svelte';
  import { fmtT } from '../lib/util';
  const c = $derived(player.current);
</script>
<footer data-autohide>
  <div class="np" role="button" tabindex="-1" onclick={() => ui.go('now')} onkeydown={() => {}}>
    <Cover tag="i" cls="" cover={c?.cover} name={c?.album ?? ''} hue={c?.hue} />
    <div style="min-width:0">
      <b>{c?.title ?? '未在播放'}</b>
      <span>{c?.artist ?? '拖入音乐文件，或在音乐库里选一首'}</span>
      {#if c}<span class="mp-fmt">{c.format}{c.bit_depth && c.sample_rate ? ` ${c.bit_depth}/${Math.round(c.sample_rate / 1000)}` : ''}</span>{/if}
    </div>
  </div>
  <div class="mid">
    <div class="ctrl">
      <!-- 占位，让播放键保持居中（播放模式按钮在右侧） -->
      <button style="visibility:hidden" tabindex="-1" aria-hidden="true"><Icon name="order" /></button>
      <button title="上一首 (Ctrl+←)" onclick={() => player.prev()}><Icon name="prev" /></button>
      <button class="pp" title={player.playing ? '暂停 (空格)' : '播放 (空格)'} onclick={() => player.toggle()}><Icon name={player.playing ? 'pause' : 'play'} /></button>
      <button title="下一首 (Ctrl+→)" onclick={() => player.next()}><Icon name="next" /></button>
      <button class:on={player.mode !== 'order'} title={`${PLAY_MODE_LABEL[player.mode]}（点击切换，Ctrl+R）`} aria-label={'播放模式：' + PLAY_MODE_LABEL[player.mode]} onclick={() => player.cycleMode()}>
        <Icon name={PLAY_MODE_ICON[player.mode]} />
      </button>
    </div>
    <div class="bar"><span>{fmtT(player.t)}</span><Slider value={player.dur ? player.t / player.dur : 0} onchange={v => player.seek(v * player.dur)} /><span>{fmtT(player.dur)}</span></div>
  </div>
  <div class="right">
    <button title="静音 (M)" onclick={() => player.toggleMute()} style="opacity:.6"><Icon name="vol" /></button>
    <Slider value={player.muted ? 0 : player.vol} onchange={v => player.setVol(v)} title="音量 (↑ / ↓)" />
    <button class:on={ui.queueOpen} title="播放队列 (Q)" onclick={() => { ui.queueOpen = !ui.queueOpen; ui.go('now'); }}><Icon name="list" /></button>
    <button title="转换当前曲目" onclick={() => { if (player.current) { ui.convertPaths = [player.current.path]; ui.go('convert'); } }}><Icon name="convert" /></button>
    <button title="均衡器 (E)" onclick={() => ui.open('eq')}><Icon name="eq" /></button>
    <button title="迷你播放器 (Ctrl+M)" onclick={() => ui.setMini(true)}><Icon name="mini" /></button>
    <button title="沉浸模式 (F)" onclick={() => ui.setImmersive()}><Icon name="expand" /></button>
  </div>
</footer>
