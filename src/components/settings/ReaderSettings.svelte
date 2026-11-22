<script lang="ts">
  // 设置页“阅读”分组：漫画 / 小说阅读器的默认翻页方式等。每本书在阅读器里改过的翻页方式和方向会单独记住
  import { onMount } from 'svelte';
  import {
    COMIC_DEFAULTS,
    COMIC_FITS,
    COMIC_FLOWS,
    NOVEL_DEFAULTS,
    NOVEL_MODES,
    PAGE_ANIMS,
    SPEED_MAX,
    SPEED_MIN,
    comicGlobal,
    novelGlobal,
    saveComicGlobal,
    saveNovelGlobal,
    type ComicPrefs,
    type NovelPrefs,
  } from '../../lib/readerprefs';

  let c = $state<ComicPrefs>({ ...COMIC_DEFAULTS });
  let n = $state<NovelPrefs>({ ...NOVEL_DEFAULTS });

  onMount(async () => {
    [c, n] = await Promise.all([comicGlobal(), novelGlobal()]);
  });

  function setC<K extends keyof ComicPrefs>(k: K, v: ComicPrefs[K]) {
    c[k] = v;
    void saveComicGlobal({ ...c });
  }

  function setN<K extends keyof NovelPrefs>(k: K, v: NovelPrefs[K]) {
    n[k] = v;
    void saveNovelGlobal({ ...n });
  }

  const num = (e: Event) => Number((e.currentTarget as HTMLInputElement).value);
</script>

<div class="card rd-set">
  <h5>阅读</h5>
  <p class="note">漫画和小说阅读器的默认设置。在阅读器里改过翻页方式或阅读方向的书会单独记住自己的设置，这里只影响其他书。</p>
  <div class="rd-cols">
    <div>
      <h6>漫画</h6>
      <div class="mp-opt col">
        <div>翻页方式</div>
        <div class="seg">
          {#each COMIC_FLOWS as f (f.id)}
            <button class:on={c.flow === f.id} title={f.desc} onclick={() => setC('flow', f.id)}>{f.name}</button>
          {/each}
        </div>
      </div>
      <div class="mp-opt">
        <div>阅读方向<small>左右翻页、左右连续滚动时有效</small></div>
        <div class="seg">
          <button class:on={c.dir === 'ltr'} onclick={() => setC('dir', 'ltr')}>从左到右</button>
          <button class:on={c.dir === 'rtl'} onclick={() => setC('dir', 'rtl')}>从右到左（日漫）</button>
        </div>
      </div>
      <div class="mp-opt">
        <div>版式<small>翻页模式下一次显示一页还是两页</small></div>
        <div class="seg">
          <button class:on={c.spread === 'single'} onclick={() => setC('spread', 'single')}>单页</button>
          <button class:on={c.spread === 'double'} onclick={() => setC('spread', 'double')}>双页</button>
        </div>
      </div>
      <label class="mp-opt">
        <div>双页时封面单独一页<small>横向的跨页大图总是单独显示</small></div>
        <input type="checkbox" class="mp-sw" checked={c.coverAlone} onchange={e => setC('coverAlone', (e.currentTarget as HTMLInputElement).checked)} />
      </label>
      <div class="mp-opt">
        <div>翻页动画</div>
        <div class="seg">
          {#each PAGE_ANIMS as a (a.id)}
            <button class:on={c.anim === a.id} onclick={() => setC('anim', a.id)}>{a.name}</button>
          {/each}
        </div>
      </div>
      <div class="mp-opt">
        <div>页面大小<small>连续滚动时“适应窗口”按“适应宽度”处理</small></div>
        <div class="seg">
          {#each COMIC_FITS as f (f.id)}
            <button class:on={c.fit === f.id} onclick={() => setC('fit', f.id)}>{f.name}</button>
          {/each}
        </div>
      </div>
      <div class="mp-opt">
        <div>连续滚动的页面宽度 {c.size}%</div>
        <input type="range" min="20" max="100" step="5" value={c.size} oninput={e => setC('size', num(e))} aria-label="连续滚动的页面宽度" />
      </div>
      <div class="mp-opt">
        <div>连续滚动的页间距 {c.gap}px</div>
        <input type="range" min="0" max="60" step="2" value={c.gap} oninput={e => setC('gap', num(e))} aria-label="连续滚动的页间距" />
      </div>
    </div>
    <div>
      <h6>小说</h6>
      <div class="mp-opt col">
        <div>翻页方式</div>
        <div class="seg">
          {#each NOVEL_MODES as m (m.id)}
            <button class:on={n.mode === m.id} title={m.desc} onclick={() => setN('mode', m.id)}>{m.name}</button>
          {/each}
        </div>
      </div>
      <div class="mp-opt">
        <div>翻页动画<small>左右翻页时有效</small></div>
        <div class="seg">
          {#each PAGE_ANIMS as a (a.id)}
            <button class:on={n.anim === a.id} onclick={() => setN('anim', a.id)}>{a.name}</button>
          {/each}
        </div>
      </div>
      <div class="mp-opt">
        <div>自动滚动速度 {n.speed}<small>每秒滚动的像素；阅读时按 ↑ / ↓ 调整，空格暂停</small></div>
        <input type="range" min={SPEED_MIN} max={SPEED_MAX} step="5" value={n.speed} oninput={e => setN('speed', num(e))} aria-label="自动滚动速度" />
      </div>
    </div>
  </div>
</div>

<style>
  .rd-set {
    grid-column: 1 / -1;
  }
  .rd-cols {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 24px;
  }
  .rd-set h6 {
    margin: 10px 0 2px;
    font-size: 13px;
    color: var(--mp-sub);
  }
  .rd-set .mp-opt.col {
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
  }
  .rd-set input[type='range'] {
    accent-color: var(--mp-accent);
  }
  @media (max-width: 900px) {
    .rd-cols {
      grid-template-columns: 1fr;
    }
  }
</style>
