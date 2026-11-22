<script lang="ts">
  // 下载列表：进度、速度、状态，暂停 / 继续 / 取消 / 重试，完成后显示整理到的路径
  import { toast } from '../../lib/ui.svelte';
  import { fetchApi, canResume, fmtBytes, isActive, STATUS_TEXT, type TaskRow } from '../../lib/fetch';

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
  const barText = (t: TaskRow) => (t.status === 'downloading' || t.status === 'seeding' ? `${Math.round(t.progress * 100)}%` : '');
</script>

{#if tasks.length === 0}
  <div class="empty" style="padding:18px">还没有下载任务。在上面搜索后点“下载”，任务会出现在这里。</div>
{/if}

{#each tasks as t (t.id)}
  <div class="dl" class:done={isDone(t.status)} style="--p:{t.progress}">
    <span title={t.title}>{t.title}</span>
    <small>
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
    </div>
  </div>
{/each}

<style>
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
