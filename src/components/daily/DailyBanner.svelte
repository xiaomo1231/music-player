<script lang="ts">
  import Cover from '../Cover.svelte';
  import Icon from '../../lib/Icon.svelte';
  import { coverCSS } from '../../lib/util';
  import { dayLabel, type DailyDoc } from '../../lib/daily';
  import type { Track } from '../../lib/api';

  let {
    doc,
    covers,
    past,
    busy,
    onplay,
    onregen,
    onsave,
  }: {
    doc: DailyDoc;
    /** 前 4 首的歌，用来拼封面 */
    covers: Track[];
    /** 正在回看以前的某一天：没有“换一批” */
    past: boolean;
    busy: boolean;
    onplay: () => void;
    onregen: () => void;
    onsave: () => void;
  } = $props();

  const hue = $derived(covers[0]?.hue ?? 210);
</script>

<div class="banner" style:background-image={coverCSS(hue, true)}>
  <div class="dmos">
    {#each covers.slice(0, 4) as t (t.id)}
      <Cover cover={t.cover} name={t.album} hue={t.hue} tag="i" />
    {/each}
  </div>
  <div>
    <span class="badge2 ai" style="justify-self:start;display:inline-block">{dayLabel(doc.day)}</span>
    <h2>{doc.title}</h2>
    <p>{doc.summary}</p>
    <div class="acts">
      <button class="mp-btn" onclick={onplay} disabled={!doc.tracks.length}><Icon name="play" /> 播放全部 · {doc.tracks.length} 首</button>
      {#if !past}
        <button class="mp-btn ghost" onclick={onregen} disabled={busy || !doc.tracks.length}>换一批</button>
      {/if}
      <button class="mp-btn ghost" onclick={onsave} disabled={!doc.tracks.length}>存为歌单</button>
    </div>
  </div>
</div>
