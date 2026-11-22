<script lang="ts">
  import { STATUS_LABEL, baseName, dirName, isAudioPath, isVideoPath, type ConvertJob } from '../../lib/convert';

  interface Actions {
    cancel: (id: number) => void;
    retry: (id: number) => void;
    undo: (id: number) => void;
    play: (j: ConvertJob) => void;
    folder: (path: string) => void;
  }

  let {
    jobs,
    speeds,
    actions,
    onclear,
  }: {
    jobs: ConvertJob[];
    speeds: Record<number, number | null>;
    actions: Actions;
    onclear: () => void;
  } = $props();

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
  <button class="mp-btn ghost" onclick={onclear}>清除已结束</button>
</div>

{#if !jobs.length}
  <div class="empty">还没有转换任务</div>
{:else}
  <div class="mp-q">
    {#each jobs as j (j.id)}
      <div class="mp-qi" style="--p:{j.progress}">
        <span title={j.input}>{baseName(j.input)}</span>
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
