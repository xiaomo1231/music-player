<script lang="ts">
  // 设置页“歌词动效”分组：样式、字号、对齐、模糊，带一个用示例歌词循环播放的小预览
  import Lyrics from '../Lyrics.svelte';
  import { invoke, inTauri } from '../../lib/api';
  import { lyricsfx, LYRIC_STYLES, LYRIC_ALIGNS } from '../../lib/lyricsfx.svelte';

  lyricsfx.load();
  const cfg = $derived(lyricsfx.cfg);

  // 在线歌词存在设置键 lyrics_online（后端读取）：总开关 + 启用的歌词源，默认全开
  const SOURCES = [
    { id: 'qq', name: 'QQ 音乐' },
    { id: 'netease', name: '网易云音乐' },
    { id: 'kugou', name: '酷狗音乐' },
    { id: 'lrclib', name: 'LRCLIB' },
  ];
  let online = $state({ enabled: true, sources: SOURCES.map(x => x.id) });
  if (inTauri) {
    invoke<{ enabled?: boolean; sources?: string[] } | null>('get_setting', { key: 'lyrics_online' })
      .then(v => {
        online = { enabled: v?.enabled ?? true, sources: Array.isArray(v?.sources) ? v.sources : online.sources };
      })
      .catch(() => {});
  }
  function saveOnline(patch: Partial<typeof online>) {
    online = { ...online, ...patch };
    if (inTauri) invoke('set_setting', { key: 'lyrics_online', value: $state.snapshot(online) }).catch(() => {});
  }
  function toggleSource(id: string, on: boolean) {
    // 保持固定顺序，后端按这个顺序决定同等条件下的优先级
    saveOnline({ sources: SOURCES.map(x => x.id).filter(x => (x === id ? on : online.sources.includes(x))) });
  }
</script>

<div class="card ly-set">
  <h5>歌词动效</h5>
  <p class="note">正在播放页和沉浸模式的歌词样式；也可以在正在播放页把鼠标移到歌词上，点右上角的 Aa 快速切换。没有逐字时间的歌词按字数估算逐字进度。</p>
  <div class="ly-set-body">
    <div class="ly-set-opts">
      <div class="ly-fxgrid ly-set-styles">
        {#each LYRIC_STYLES as s}
          <button class:on={cfg.style === s.id} onclick={() => lyricsfx.set({ style: s.id })}><b>{s.name}</b><small>{s.desc}</small></button>
        {/each}
      </div>
      <div class="mp-opt">
        <div>字号 {Math.round(cfg.size * 100)}%</div>
        <input type="range" min="0.7" max="1.6" step="0.05" value={cfg.size} aria-label="歌词字号" oninput={e => lyricsfx.set({ size: +(e.currentTarget as HTMLInputElement).value })} />
      </div>
      <div class="mp-opt">
        <div>对齐<small>竖排时对应上 / 中 / 下</small></div>
        <div class="seg">
          {#each LYRIC_ALIGNS as a}
            <button class:on={cfg.align === a.id} onclick={() => lyricsfx.set({ align: a.id })}>{a.name}</button>
          {/each}
        </div>
      </div>
      <label class="mp-opt">
        <div>模糊非当前行<small>关闭后其他行只变淡，不模糊</small></div>
        <input type="checkbox" class="mp-sw" checked={cfg.blur} onchange={e => lyricsfx.set({ blur: (e.currentTarget as HTMLInputElement).checked })} />
      </label>
      <label class="mp-opt">
        <div>显示翻译<small>歌词带译文时显示在原文下方（双语 .lrc 里时间相同的第二行就是译文）</small></div>
        <input type="checkbox" class="mp-sw" checked={cfg.tr} onchange={e => lyricsfx.set({ tr: (e.currentTarget as HTMLInputElement).checked })} />
      </label>
      <label class="mp-opt">
        <div>显示罗马音<small>日语、韩语歌词带罗马音时显示</small></div>
        <input type="checkbox" class="mp-sw" checked={cfg.roma} onchange={e => lyricsfx.set({ roma: (e.currentTarget as HTMLInputElement).checked })} />
      </label>
      <label class="mp-opt">
        <div>在线获取歌词<small>本地没有 .lrc 和内嵌歌词时在线查找，优先逐字歌词和带译文的版本，结果缓存在本地</small></div>
        <input type="checkbox" class="mp-sw" checked={online.enabled} onchange={e => saveOnline({ enabled: (e.currentTarget as HTMLInputElement).checked })} />
      </label>
      {#if online.enabled}
        <div class="mp-opt">
          <div>歌词源<small>QQ 音乐、网易云、酷狗用的是客户端的非官方接口，偶尔会失效</small></div>
          <div class="seg">
            {#each SOURCES as src}
              <button class:on={online.sources.includes(src.id)} onclick={() => toggleSource(src.id, !online.sources.includes(src.id))}>{src.name}</button>
            {/each}
          </div>
        </div>
      {/if}
    </div>
    <div class="ly-set-preview" aria-hidden="true">
      <Lyrics cls="lyr ly-preview" pos={0.5} demo />
    </div>
  </div>
</div>

<style>
  .ly-set {
    grid-column: 1 / -1;
  }
  .ly-set-body {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr);
    gap: 18px;
    align-items: stretch;
  }
  .ly-set input[type='range'] {
    accent-color: var(--mp-accent);
  }
  .ly-set-styles {
    grid-template-columns: repeat(3, 1fr);
    margin-bottom: 10px;
  }
  .ly-set-styles button {
    text-align: left;
    padding: 8px 10px;
  }
  .ly-set-styles b {
    display: block;
    font-size: 13px;
  }
  .ly-set-styles small {
    display: block;
    font-size: 11px;
    color: var(--mp-sub);
    margin-top: 2px;
  }
  .ly-set-preview {
    position: relative;
    min-height: 230px;
    border-radius: 12px;
    border: 1px solid var(--edge);
    background: rgba(0, 0, 0, 0.22);
    padding: 0 16px;
    overflow: hidden;
    pointer-events: none;
  }
  .ly-set-preview :global(.lyr.ly-preview) {
    height: 100%;
    min-height: 230px;
    font-size: calc(22px * var(--ly-size, 1));
  }
  @media (max-width: 900px) {
    .ly-set-body {
      grid-template-columns: 1fr;
    }
  }
</style>
