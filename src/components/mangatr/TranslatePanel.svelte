<script lang="ts">
  // 漫画 AI 翻译侧栏：翻译 / 对照切换、逐句校对、术语表、导出（原型的 .rside / .tr / .gl 样式）
  import { untrack } from 'svelte';
  import { mangatr, type Estimate, type Term } from '../../lib/mangatr.svelte';
  import { inTauri, invoke } from '../../lib/api';
  import { ui, toast } from '../../lib/ui.svelte';

  let { comicId, page, pages, mode, onmode }: {
    comicId: number;
    page: number;
    pages: number;
    mode: 'orig' | 'tr' | 'both';
    onmode: (m: 'orig' | 'tr' | 'both') => void;
  } = $props();

  /** null 未检查；true 已配置；false 未配置 */
  let aiReady = $state<boolean | null>(null);
  let est = $state<Estimate | null>(null);
  let exporting = $state(false);
  let gl = $state<Term[]>([]);

  $effect(() => {
    const id = comicId;
    untrack(() => void mangatr.load(id));
  });

  // 换书或重新载入时同步术语表编辑区（编辑中的内容不随译文刷新被覆盖）
  $effect(() => {
    void mangatr.loadSeq;
    untrack(() => {
      gl = mangatr.book.glossary.map((t) => ({ ...t }));
    });
  });

  // 从 AI 设置回来后重新检查
  $effect(() => {
    void ui.modal;
    void checkAi();
  });

  async function checkAi() {
    if (!inTauri) {
      aiReady = null;
      return;
    }
    try {
      const s = await invoke<{ config: { base_url: string }; has_key: boolean }>('ai_get_config');
      aiReady = s.has_key || /localhost|127\.0\.0\.1/.test(s.config.base_url);
    } catch {
      aiReady = null;
    }
  }

  function fail(e: unknown) {
    const msg = String(e);
    if (msg.includes('还没有配置 AI')) aiReady = false;
    toast(msg);
  }

  const blocks = $derived(mangatr.comicId === comicId ? mangatr.blocksOf(page) : []);
  const textBlocks = $derived(blocks.filter((b) => !b.skipped));
  const skippedCount = $derived(blocks.filter((b) => b.skipped).length);
  const anyTr = $derived(Array.from({ length: pages }, (_, p) => p).some((p) => mangatr.hasTr(p)));

  async function translatePage() {
    try {
      await mangatr.translatePage(page);
      toast('本页翻译完成，可在右侧逐句修改');
    } catch (e) {
      fail(e);
    }
  }

  async function askRange() {
    if (pages <= 0) return;
    try {
      est = await mangatr.estimate(0, pages - 1);
    } catch (e) {
      fail(e);
    }
  }

  async function startRange() {
    const e = est;
    est = null;
    if (!e) return;
    try {
      await mangatr.translateRange(0, pages - 1);
      toast('开始翻译整话，完成后会提示');
    } catch (err) {
      fail(err);
    }
  }

  async function cancel() {
    try {
      await mangatr.cancel();
      toast('正在取消…');
    } catch (e) {
      fail(e);
    }
  }

  async function retranslate(id: string) {
    try {
      await mangatr.retranslate(page, id);
    } catch (e) {
      fail(e);
    }
  }

  async function saveGlossary(applyAll: boolean) {
    try {
      const n = await mangatr.setGlossary(gl, applyAll);
      toast(applyAll ? `已应用到全书，替换了 ${n} 处译文` : '术语表已保存');
    } catch (e) {
      fail(e);
    }
  }

  async function exportBook() {
    exporting = true;
    try {
      const path = await mangatr.exportCbz(pages);
      toast('已导出：' + path);
    } catch (e) {
      fail(e);
    } finally {
      exporting = false;
    }
  }

  const progressText = $derived.by(() => {
    const p = mangatr.progress;
    if (!p || p.comic_id !== comicId) return '';
    return `第 ${p.page + 1} 页 · ${p.message}`;
  });
  const progressPct = $derived.by(() => {
    const p = mangatr.progress;
    return p && p.total > 0 ? Math.min(100, Math.round((p.done / p.total) * 100)) : 0;
  });
</script>

<div class="rside mtr-panel">
  <h5>AI 翻译 <span class="mtr-sub">第 {page + 1} / {pages} 页</span></h5>

  {#if aiReady === false}
    <div class="note mtr-note">
      <span>还没有配置 AI 服务。翻译需要先填写接口地址和密钥。</span>
      <button class="rbtn pri" onclick={() => ui.open('ai')}>去配置 AI 服务</button>
    </div>
  {/if}

  <div class="mtr-row">
    <button class="rbtn pri" disabled={mangatr.busy || mangatr.running || aiReady === false} onclick={translatePage}>
      {mangatr.busy ? '翻译中…' : '翻译本页'}
    </button>
    {#if mangatr.running}
      <button class="rbtn" onclick={cancel}>取消</button>
    {:else}
      <button class="rbtn" disabled={mangatr.busy || aiReady === false} onclick={askRange}>翻译本话</button>
    {/if}
  </div>

  {#if est}
    <div class="note mtr-note">
      <span>本话共 {est.pages} 页，预计用量约 {est.total.toLocaleString()} token。{est.note}</span>
      <div class="mtr-row">
        <button class="rbtn pri" onclick={startRange}>开始翻译</button>
        <button class="rbtn" onclick={() => (est = null)}>先不翻</button>
      </div>
    </div>
  {/if}

  {#if mangatr.running && progressText}
    <div class="mtr-prog">
      <div class="mtr-bar"><i style="width:{progressPct}%"></i></div>
      <small>{progressText}</small>
    </div>
  {/if}

  <div class="mtr-row">
    <button class="rbtn" class:on={mode === 'orig'} onclick={() => onmode('orig')}>原文</button>
    <button class="rbtn" class:on={mode === 'tr'} onclick={() => onmode('tr')}>译文</button>
    <button class="rbtn" class:on={mode === 'both'} onclick={() => onmode('both')}>对照</button>
  </div>

  <h5>逐句校对 <span class="mtr-sub">改这里，画面同步更新</span></h5>
  {#if textBlocks.length === 0}
    <div class="empty mtr-empty">
      {blocks.length === 0 ? '这一页还没有翻译。点“翻译本页”识别并翻译。' : '这一页只有拟声词或画面文字，第一版不翻译。'}
    </div>
  {:else}
    {#each textBlocks as b (b.id)}
      <div class="tr" class:mtr-sel={mangatr.selected === b.id}>
        <small>{b.order}. {b.src}</small>
        <input
          value={b.tr}
          placeholder="…"
          oninput={(e) => mangatr.editTr(page, b.id, e.currentTarget.value)}
        />
        {#if b.alt && b.alt.length}
          <div class="mtr-alt">
            <span class="mtr-sub">备选：</span>
            {#each b.alt as a}
              <button class="mtr-chip" onclick={() => mangatr.commitTr(page, b.id, a)}>{a}</button>
            {/each}
          </div>
        {/if}
        <div class="mtr-row">
          <button class="rbtn" onclick={() => (mangatr.selected = b.id)}>在画面中定位</button>
          <button class="rbtn" disabled={mangatr.busy} onclick={() => retranslate(b.id)}>带上下文重译</button>
        </div>
      </div>
    {/each}
    {#if skippedCount > 0}
      <small class="mtr-sub">另有 {skippedCount} 处拟声词或画面文字未翻译。</small>
    {/if}
  {/if}

  <h5>术语表 <span class="mtr-sub">保证人名、地名全书一致</span></h5>
  {#each gl as t, i (i)}
    <div class="mtr-gl">
      <input bind:value={t.src} placeholder="原文" />
      <span>→</span>
      <input bind:value={t.tr} placeholder="译文" />
      <button class="rbtn" title="删除这一条" onclick={() => gl.splice(i, 1)}>删</button>
    </div>
  {/each}
  <div class="mtr-row">
    <button class="rbtn" onclick={() => gl.push({ src: '', tr: '' })}>＋ 添加</button>
    <button class="rbtn" onclick={() => saveGlossary(false)}>保存</button>
    <button class="rbtn pri" onclick={() => saveGlossary(true)}>应用到全书</button>
  </div>

  <div class="mtr-row">
    <button class="rbtn" disabled={exporting || !anyTr} onclick={exportBook}>
      {exporting ? '导出中…' : '导出为新文件（保留原版）'}
    </button>
  </div>
</div>

<style>
  .mtr-panel {
    min-width: 0;
  }
  .mtr-sub {
    color: #888;
    font-weight: 400;
    font-size: 11.5px;
  }
  .mtr-row {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    align-items: center;
  }
  .mtr-note {
    display: grid;
    gap: 8px;
    font-size: 12px;
    color: #ccc;
    padding: 8px 10px;
    border-radius: 10px;
    background: #ffffff0c;
  }
  .mtr-empty {
    padding: 14px;
    font-size: 12.5px;
  }
  .mtr-prog {
    display: grid;
    gap: 4px;
  }
  .mtr-bar {
    height: 6px;
    border-radius: 3px;
    background: #ffffff1a;
    overflow: hidden;
  }
  .mtr-bar i {
    display: block;
    height: 100%;
    background: linear-gradient(90deg, #ff3d6e, #ff8a3d);
  }
  .mtr-sel {
    border-color: #ff3d6e !important;
  }
  .mtr-alt {
    display: flex;
    gap: 5px;
    flex-wrap: wrap;
    align-items: center;
    font-size: 12px;
  }
  .mtr-chip {
    background: #ffffff12;
    border: 1px dashed #ffffff30;
    border-radius: 7px;
    color: #eee;
    padding: 2px 8px;
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .mtr-gl {
    display: grid;
    grid-template-columns: 1fr auto 1fr auto;
    gap: 6px;
    align-items: center;
    font-size: 12px;
  }
  .mtr-gl input {
    background: #ffffff0c;
    border: 1px solid #ffffff1c;
    border-radius: 7px;
    color: #eee;
    padding: 4px 7px;
    font: inherit;
    min-width: 0;
  }
</style>
