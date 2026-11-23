<script lang="ts">
  // 按漫画排版规则导出新 CBZ：另存为新文件 / 替换原文件（原文件进回收站）
  import Modal from '../Modal.svelte';
  import { listen } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { comicExport, type ComicExportProgress, type ComicExportReport } from '../../lib/emcomic';

  let {
    ids,
    title,
    ruleName = '',
    ondone,
    onclose,
  }: {
    ids: number[];
    /** 一本时显示书名 */
    title?: string;
    ruleName?: string;
    ondone?: (r: ComicExportReport, replace: boolean) => void;
    onclose: () => void;
  } = $props();

  let busy = $state(false);
  let prog = $state<ComicExportProgress | null>(null);
  let report = $state.raw<ComicExportReport | null>(null);

  $effect(() => {
    let off: (() => void) | undefined;
    void listen<ComicExportProgress>('em://comic-export', p => {
      if (busy) prog = p;
    }).then(f => (off = f));
    return () => off?.();
  });

  async function run(replace: boolean) {
    busy = true;
    prog = null;
    try {
      const r = await comicExport(ids, replace);
      report = r;
      toast(r.failed.length ? `已导出 ${r.outputs.length} 本，${r.failed.length} 本没导出` : `已导出 ${r.outputs.length} 本`);
      ondone?.(r, replace);
    } catch (e) {
      toast(String(e));
      report = { outputs: [], failed: String(e).split('\n') };
    }
    busy = false;
  }

  const pct = $derived(prog && prog.total ? ((prog.done + (prog.pages ? prog.page / prog.pages : 0)) / prog.total) * 100 : 0);
  const close = () => !busy && onclose();
</script>

<Modal title="导出排版后的 CBZ" onclose={close}>
  {#if report}
    {#if report.outputs.length}
      <p class="note">已保存：</p>
      <ul class="ce-list">
        {#each report.outputs as p (p)}<li>{p}</li>{/each}
      </ul>
    {/if}
    {#if report.failed.length}
      <p class="note">没导出：</p>
      <ul class="ce-list bad">
        {#each report.failed as f, i (i)}<li>{f}</li>{/each}
      </ul>
    {/if}
    <div class="ce-acts"><button class="mp-btn" onclick={onclose}>完成</button></div>
  {:else if busy}
    <p class="note">正在导出{prog?.current ? `「${prog.current}」` : ''}{prog && prog.total > 1 ? `（${prog.done + 1} / ${prog.total}）` : ''}…</p>
    <div class="ce-bar"><i style="width:{pct}%"></i></div>
  {:else}
    <p class="note">
      {title ? `「${title}」` : `选中的 ${ids.length} 本`}按{ruleName ? `规则「${ruleName}」` : '各自适用的规则'}拆页、裁边、跳过页面，再按规则里的导出设置转换，生成新的 CBZ（含 ComicInfo.xml）。
      图片文件夹只能另存为新文件。
    </p>
    <div class="ce-acts">
      <button class="mp-btn ghost" onclick={onclose}>取消</button>
      <button class="mp-btn ghost" onclick={() => run(true)}>替换原文件（原文件进回收站）</button>
      <button class="mp-btn" onclick={() => run(false)}>另存为新文件</button>
    </div>
  {/if}
</Modal>

<style>
  .note {
    color: var(--mp-sub);
    font-size: 12.5px;
    line-height: 1.6;
    margin: 4px 0 10px;
  }
  .ce-acts {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    flex-wrap: wrap;
    margin-top: 12px;
  }
  .ce-bar {
    height: 6px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.1);
    overflow: hidden;
  }
  .ce-bar i {
    display: block;
    height: 100%;
    background: var(--mp-accent, #ff6a55);
    transition: width 0.2s;
  }
  .ce-list {
    margin: 0;
    padding-left: 18px;
    font-size: 12.5px;
    word-break: break-all;
    max-height: 220px;
    overflow: auto;
  }
  .ce-list.bad {
    color: #ff8a8a;
  }
</style>
