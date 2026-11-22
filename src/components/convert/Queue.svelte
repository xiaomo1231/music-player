<script lang="ts">
  import { untrack } from 'svelte';
  import { STATUS_LABEL, baseName, dirName, isAudioPath, isVideoPath, type ConvertJob } from '../../lib/convert';
  import { invoke } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { confirmDlg, type MenuEntry } from '../../lib/items.svelte';
  import { Selection, noTextSelect } from '../../lib/select.svelte';
  import BatchBar from '../BatchBar.svelte';
  import SelCheck from '../SelCheck.svelte';

  interface Actions {
    cancel: (id: number) => void;
    retry: (id: number) => void;
    undo: (id: number) => void;
    play: (j: ConvertJob) => void;
    folder: (path: string) => void;
    remove: (id: number) => void;
  }

  let {
    jobs,
    speeds,
    actions,
    onclear,
    onremoved,
  }: {
    jobs: ConvertJob[];
    speeds: Record<number, number | null>;
    actions: Actions;
    onclear: () => void;
    /** 批量删除记录之后（重新读取列表） */
    onremoved?: () => void;
  } = $props();

  // 多选删除记录：进行中 / 排队中的任务不能选
  const sel = new Selection();
  const removable = (j: ConvertJob) => j.status !== 'queued' && j.status !== 'running';
  const order = $derived(jobs.filter(removable).map(j => j.id));
  $effect(() => { const o = order; untrack(() => sel.prune(o)); });
  function rowClick(e: MouseEvent, j: ConvertJob) {
    if (!removable(j) || (e.target as HTMLElement).closest('button')) return;
    sel.click(e, j.id, order);
  }
  async function removeSelected() {
    const ids = sel.ids(order);
    if (!(await confirmDlg('删除记录', `删除选中的 ${ids.length} 条转换记录？只删除记录，不动任何文件。`, { ok: '删除记录' }))) return;
    try {
      for (const id of ids) await invoke('convert_remove', { id });
      toast(`已删除 ${ids.length} 条记录`);
    } catch (e) {
      toast(String(e));
    }
    sel.clear();
    onremoved?.();
  }
  const entries: MenuEntry[] = [{ label: '删除记录', danger: true, run: removeSelected }];

  function statusText(j: ConvertJob): string {
    if (j.status === 'running') {
      const sp = speeds[j.id];
      return `${Math.round(j.progress * 100)}%${sp ? ` · ${sp.toFixed(1)}×` : ''}`;
    }
    return STATUS_LABEL[j.status];
  }

  function resultText(j: ConvertJob): string {
    if (j.status === 'undone') return '已撤销替换，原文件已还原';
    switch (j.after) {
      case 'lib':
        return '✓ 已加入媒体库';
      case 'replace':
        return '✓ 已替换原文件，原文件在回收站';
      case 'side':
        return '✓ 已保存到原文件旁边';
      default:
        return `✓ 已输出到 ${j.output ? dirName(j.output) : ''}`;
    }
  }

  const canPlay = (j: ConvertJob) => !!j.output && (isAudioPath(j.output) || isVideoPath(j.output));
</script>

<div class="qhead">
  <h5>队列 <span class="badge2">{jobs.length}</span></h5>
  <span class="qacts">
    {#if order.length}<button class="mp-btn ghost" class:on={sel.on} onclick={() => (sel.on ? sel.clear() : (sel.mode = true))}>{sel.on ? '退出多选' : '多选'}</button>{/if}
    <button class="mp-btn ghost" onclick={onclear}>清除已结束</button>
  </span>
</div>

<svelte:window onkeydown={e => order.length && sel.keys(e, () => order)} />

{#if sel.on}
  <BatchBar {sel} unit="条" total={order.length} onall={() => sel.selectAll(order)} {entries} hint="单击勾选，Shift 连选，Ctrl+A 全选（进行中的任务不能选）" />
{/if}

{#if !jobs.length}
  <div class="empty">还没有转换任务</div>
{:else}
  <div class="mp-q">
    {#each jobs as j (j.id)}
      <div class="mp-qi" class:picked={sel.has(j.id)} class:pickable={sel.on && removable(j)} style="--p:{j.progress}"
        role="presentation" onclick={e => rowClick(e, j)} onmousedown={noTextSelect}>
        <span title={j.input} class="qname">{#if sel.on && removable(j)}<SelCheck checked={sel.has(j.id)} onclick={e => (e.shiftKey ? sel.range(order, j.id, true) : sel.toggle(j.id))} />{/if}<span class="ell">{baseName(j.input)}</span></span>
        <small class="mp-qs">{statusText(j)}</small>
        <small title={j.output ?? j.out_dir ?? ''}>{j.preset_name}</small>
        <span></span>
        <div class="mp-pb"><i></i></div>
        <div class="res">
          {#if j.status === 'done' || j.status === 'undone'}
            <span>{resultText(j)}</span>
          {:else if j.status === 'failed'}
            <span class="err">{j.error ?? '转换失败'}</span>
          {:else if j.status === 'canceled'}
            <span>已取消，半成品已删除</span>
          {/if}
          <span class="acts">
            {#if j.status === 'queued' || j.status === 'running'}
              <button onclick={() => actions.cancel(j.id)}>取消</button>
            {/if}
            {#if (j.status === 'failed' || j.status === 'canceled') && !j.can_undo}
              <button onclick={() => actions.retry(j.id)}>重试</button>
            {/if}
            {#if j.status === 'done' && canPlay(j)}
              <button onclick={() => actions.play(j)}>{j.output && isAudioPath(j.output) ? '播放' : '观看'}</button>
            {/if}
            {#if (j.status === 'done' || j.status === 'undone') && j.output}
              <button onclick={() => actions.folder(j.output ?? '')}>在文件夹中显示</button>
            {/if}
            {#if j.can_undo}
              <button onclick={() => actions.undo(j.id)}>撤销替换</button>
            {/if}
            {#if j.status !== 'queued' && j.status !== 'running'}
              <button title="只删除这条记录，不动任何文件" onclick={() => actions.remove(j.id)}>删除记录</button>
            {/if}
          </span>
        </div>
      </div>
    {/each}
  </div>
{/if}

<style>
  .qhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin: 18px 0 10px;
  }
  .qacts { display: flex; gap: 6px; }
  .qacts .on { border-color: var(--mp-accent); color: var(--mp-accent); }
  .qname { display: flex; align-items: center; gap: 8px; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ell { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pickable { cursor: pointer; }
  .picked { background: color-mix(in srgb, var(--mp-accent) 15%, transparent); }
  .qhead h5 {
    margin: 0;
    font-size: 14px;
  }
  .res {
    grid-column: 1 / -1;
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
    font-size: 12px;
    color: var(--mp-sub);
  }
  .res .acts {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-left: auto;
  }
  .res button {
    border: 1px solid var(--mp-line);
    background: none;
    color: inherit;
    border-radius: 999px;
    padding: 3px 10px;
    font: inherit;
    font-size: 12px;
    cursor: pointer;
  }
  .err {
    color: #ff8a8a;
    word-break: break-all;
  }
  .empty {
    padding: 20px;
    text-align: center;
    color: var(--mp-sub);
    font-size: 13px;
  }
</style>
