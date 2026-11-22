<script lang="ts">
  import Modal from './Modal.svelte';
  import { player, EQF, EQ_PRESETS } from '../lib/player.svelte';
  import { ui } from '../lib/ui.svelte';
  import { engine } from '../lib/engine.svelte';
  let gains = $state([...player.eq]);
  let preset = $state('');
  function set(i: number, v: number) {
    gains[i] = v;
    player.setEq([...gains]);
    preset = '';
  }
  function usePreset(k: string) {
    gains = [...EQ_PRESETS[k]];
    player.setEq([...gains]);
    preset = k;
  }
</script>

<Modal title="均衡器与输出" onclose={() => ui.close()}>
  <p class="mp-desc">10 段均衡器实时生效。拖动推子或者选一个预设。</p>
  <div class="mp-chips">
    {#each Object.keys(EQ_PRESETS) as k}<button class="mp-chip" class:on={preset === k} onclick={() => usePreset(k)}>{k}</button>{/each}
  </div>
  <div class="mp-eq">
    {#each EQF as f, i}
      <label class="mp-eqb">
        <input type="range" min="-12" max="12" step="1" value={gains[i]} oninput={e => set(i, +(e.target as HTMLInputElement).value)} />
        <span>{f >= 1000 ? f / 1000 + 'k' : f}</span>
      </label>
    {/each}
  </div>
  <label class="mp-opt"><div>播放速度<small>变速不变调</small></div>
    <select value={String(player.speed)} onchange={e => player.setSpeed(+(e.target as HTMLSelectElement).value)}>
      {#each [0.5, 0.75, 1, 1.25, 1.5, 2] as v}<option value={String(v)}>{v}×</option>{/each}
    </select>
  </label>
  <label class="mp-opt"><div>WASAPI 独占输出<small>绕过系统混音器，按文件原始采样率比特完美输出。{engine.mpv ? '' : '需要 mpv 播放引擎（设置 → 播放引擎）'}</small></div>
    <input type="checkbox" class="mp-sw" disabled={!engine.mpv} checked={engine.exclusive} onchange={e => engine.setExclusive((e.target as HTMLInputElement).checked)} />
  </label>
  <label class="mp-opt"><div>响度均衡 (ReplayGain)<small>读取文件里的 ReplayGain 信息，不同来源的歌曲保持一致音量。{engine.mpv ? '' : '需要 mpv 播放引擎'}</small></div>
    <input type="checkbox" class="mp-sw" disabled={!engine.mpv} checked={engine.replaygain} onchange={e => engine.setReplaygain((e.target as HTMLInputElement).checked)} />
  </label>
</Modal>
