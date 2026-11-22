<script lang="ts">
  import { untrack } from 'svelte';
  import { invoke } from '../../lib/api';
  import { confirmDlg, type MenuEntry } from '../../lib/items.svelte';
  import { Selection, noTextSelect } from '../../lib/select.svelte';
  import BatchBar from '../BatchBar.svelte';
  import SelCheck from '../SelCheck.svelte';
  // 下载列表：进度、速度、状态，暂停 / 继续 / 取消 / 重试，完成后显示整理到的路径
  import { toast } from '../../lib/ui.svelte';
  import { fetchApi, canResume, fmtBytes, isActive, MEDIA_TEXT, STATUS_TEXT, type TaskRow } from '../../lib/fetch';

  let { tasks, onchange }: { tasks: TaskRow[]; onchange?: () => void } = $props();

  async function act(fn: () => Promise<void>, ok?: string) {
    try {
      await fn();
      if (ok) toast(ok);
      onchange?.();
    } catch (err) {
      toast(String(err));
    }
  }

  const isDone = (s: string) => s === 'done' || s === 'manual';

  // 多选删除记录：进行中的任务不能选
  const sel = new Selection();
  const removable = (t: TaskRow) => !isActive(t.status);
  const order = $derived(tasks.filter(removable).map(t => t.id));
  $effect(() => { const o = order; untrack(() => sel.prune(o)); });
  function rowClick(e: MouseEvent, t: TaskRow) {
    if (!removable(t) || (e.target as HTMLElement).closest('button')) return;
    sel.click(e, t.id, order);
  }
  async function removeSelected() {
    const ids = sel.ids(order);
    if (!(await confirmDlg('删除记录', `删除选中的 ${ids.length} 条下载记录？只从列表里删掉记录，已整理入库的文件不受影响。`, { ok: '删除记录' }))) return;
    await act(async () => { for (const id of ids) await invoke('fetch_remove_task', { id }); }, `已删除 ${ids.length} 条记录`);
    sel.clear();
  }
  const entries: MenuEntry[] = [{ label: '删除记录', danger: true, run: removeSelected }];
  const barText = (t: TaskRow) => (t.status === 'downloading' || t.status === 'seeding' ? `${Math.round(t.progress * 100)}%` : '');
</script>

{#if tasks.length === 0}
  <div class="empty" style="padding:18px">还没有下载任务。在上面搜索后点“下载”，任务会出现在这里。</div>
{/if}

<svelte:window onkeydown={e => order.length && sel.keys(e, () => order)} />

{#if order.length}
  {#if sel.on}
    <BatchBar {sel} unit="条" total={order.length} onall={() => sel.selectAll(order)} {entries} hint="单击勾选，Shift 连选，Ctrl+A 全选（进行中的任务不能选）" />
  {:else}
    <div class="dlhead"><button class="mp-chip" onclick={() => (sel.mode = true)}>多选</button></div>
  {/if}
{/if}

{#each tasks as t (t.id)}
  <div class="dl" class:done={isDone(t.status)} class:picked={sel.has(t.id)} class:pickable={sel.on && removable(t)} style="--p:{t.progress}"
    role="presentation" onclick={e => rowClick(e, t)} onmousedown={noTextSelect}>
    <span title={t.title} class="dname">{#if sel.on && removable(t)}<SelCheck checked={sel.has(t.id)} onclick={e => (e.shiftKey ? sel.range(order, t.id, true) : sel.toggle(t.id))} />{/if}<span class="ell">{t.title}</span></span>
    <small>
      {#if MEDIA_TEXT[t.media_kind]}{MEDIA_TEXT[t.media_kind]} ·{/if}
      {STATUS_TEXT[t.status] ?? t.status}
      {#if t.status === 'downloading' && t.speed > 0}· {fmtBytes(t.speed)}/s{/if}
      {#if t.size > 0}· {fmtBytes(t.size)}{/if}
      {#if barText(t)}· {barText(t)}{/if}
    </small>
    <div class="mp-pb"><i></i></div>
    {#if isDone(t.status) && t.save_path}
      <code>→ {t.save_path}</code>
    {/if}
    {#if t.error}
      <div class="err">{t.error}</div>
    {/if}
    <div class="acts">
      {#if t.status === 'downloading' || t.status === 'queued'}
        <button class="mp-btn ghost" onclick={() => act(() => fetchApi.pause(t.id), '已暂停')}>暂停</button>
      {/if}
      {#if canResume(t.status)}
        <button class="mp-btn ghost" onclick={() => act(() => fetchApi.resume(t.id))}>{t.status === 'failed' ? '重试' : '继续'}</button>
      {/if}
      {#if isActive(t.status) || canResume(t.status)}
        <button class="mp-btn ghost" onclick={() => act(() => fetchApi.cancel(t.id, false), '已取消')}>取消</button>
        <button class="mp-btn ghost" title="取消并删除已下载的文件" onclick={() => act(() => fetchApi.cancel(t.id, true), '已取消并删除文件')}>删除</button>
      {/if}
      {#if !isActive(t.status)}
        <button class="mp-btn ghost" title="只从列表里删掉这条记录，已整理入库的文件不受影响" onclick={() => act(async () => { await invoke('fetch_remove_task', { id: t.id }); }, '已删除记录')}>删除记录</button>
      {/if}
    </div>
  </div>
{/each}

<style>
  .dlhead { display: flex; justify-content: flex-end; margin: 0 0 6px; }
  .dname { display: flex; align-items: center; gap: 8px; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ell { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pickable { cursor: pointer; }
  .picked { background: color-mix(in srgb, var(--mp-accent) 15%, transparent); }
  .dl .acts {
    grid-column: 1 / -1;
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .dl .err {
    grid-column: 1 / -1;
    font-size: 11.5px;
    color: #ff9b9b;
    line-height: 1.5;
    word-break: break-all;
  }
</style>
