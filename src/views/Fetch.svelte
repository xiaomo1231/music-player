<script lang="ts">
  // 自动获取页：左边“我的来源”，右边“搜索并下载”；下载列表放在右侧抽屉里（顶部按钮显示进行中数量和总进度）；设置放在弹窗里
  import { onMount } from 'svelte';
  import Icon from '../lib/Icon.svelte';
  import { listen, inTauri } from '../lib/api';
  import { toast } from '../lib/ui.svelte';
  import { fetchApi, isActive, type TaskRow } from '../lib/fetch';
  import Sources from '../components/fetch/Sources.svelte';
  import Search from '../components/fetch/Search.svelte';
  import Downloads from '../components/fetch/Downloads.svelte';
  import FetchSettings from '../components/fetch/FetchSettings.svelte';
  import Bili from '../components/fetch/Bili.svelte';

  let tasks = $state<TaskRow[]>([]);
  let settingsOpen = $state(false);
  let drawer = $state(false);
  /** 刚加入下载时按钮闪一下 */
  let pulse = $state(false);
  let pulseTimer: ReturnType<typeof setTimeout> | undefined;

  const active = $derived(tasks.filter(t => isActive(t.status)));
  const downloading = $derived(active.filter(t => t.status === 'downloading' || t.status === 'queued'));
  /** 进行中任务的平均进度（0–1） */
  const totalPct = $derived(downloading.length ? downloading.reduce((a, t) => a + Math.min(1, Math.max(0, t.progress)), 0) / downloading.length : 0);
  const failed = $derived(tasks.filter(t => t.status === 'failed').length);

  async function queued() {
    await loadTasks();
    pulse = false;
    clearTimeout(pulseTimer);
    requestAnimationFrame(() => (pulse = true));
    pulseTimer = setTimeout(() => (pulse = false), 1200);
  }

  async function loadTasks() {
    if (!inTauri) return;
    try {
      tasks = await fetchApi.tasks();
    } catch (err) {
      toast(String(err));
    }
  }

  onMount(() => {
    loadTasks();
    const offs: Array<() => void> = [];
    listen<TaskRow>('fetch://progress', t => {
      const i = tasks.findIndex(x => x.id === t.id);
      if (i >= 0) tasks[i] = t;
      else tasks = [t, ...tasks];
    }).then(f => offs.push(f));
    listen<{ query: string; titles: string[] }>('fetch://new', p => {
      toast(`追更：${p.query} 有 ${p.titles.length} 集新内容，已加入下载`);
    }).then(f => offs.push(f));
    return () => offs.forEach(f => f());
  });
</script>

<section class="lib on">
  <header>
    <div>
      <h2>自动获取</h2>
      <p>添加你自己的来源，搜一下、点一下，自动下载整季或整张专辑，并整理进媒体库</p>
    </div>
    <div class="acts">
      <button class="mp-btn ghost dlbtn" class:pulse class:on={drawer} title="下载列表" onclick={() => (drawer = !drawer)}>
        <Icon name="download" /> 下载
        {#if downloading.length}<span class="badge2">{downloading.length} 进行中 · {Math.round(totalPct * 100)}%</span>
        {:else if failed}<span class="badge2 no">{failed} 失败</span>
        {:else if tasks.length}<span class="badge2">{tasks.length}</span>{/if}
        {#if downloading.length}<i class="dlbar" style:width="{totalPct * 100}%"></i>{/if}
      </button>
      <button class="mp-btn ghost" onclick={() => (settingsOpen = true)}><Icon name="gear" /> 设置</button>
    </div>
  </header>

  <div class="get" style="margin-top:14px">
    <div class="card">
      <Sources onchange={loadTasks} />
    </div>
    <div class="stack">
      <div class="card">
        <Bili onqueued={queued} />
      </div>
      <div class="card">
        <Search onqueued={queued} {tasks} />
      </div>
    </div>
  </div>
</section>

{#if drawer}
  <button class="scrim" aria-label="关闭下载列表" onclick={() => (drawer = false)}></button>
  <aside class="drawer" aria-label="下载列表">
    <div class="dhead">
      <h5>下载列表 <span class="badge2">{tasks.length}</span></h5>
      {#if downloading.length}<small>{downloading.length} 个进行中 · 总进度 {Math.round(totalPct * 100)}%</small>{/if}
      <span class="grow"></span>
      <button class="mp-btn ghost" onclick={() => (drawer = false)}>收起</button>
    </div>
    <div class="dbody">
      <Downloads {tasks} onchange={loadTasks} />
    </div>
  </aside>
{/if}

<svelte:window onkeydown={e => drawer && e.key === 'Escape' && (drawer = false)} />

{#if settingsOpen}
  <FetchSettings onclose={() => (settingsOpen = false)} onsaved={loadTasks} />
{/if}

<style>
  .dlbtn {
    position: relative;
    overflow: hidden;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .dlbtn.on {
    border-color: var(--mp-accent);
  }
  .dlbar {
    position: absolute;
    left: 0;
    bottom: 0;
    height: 2px;
    background: var(--mp-accent);
    transition: width 0.4s;
  }
  .dlbtn.pulse {
    animation: dlpulse 1.1s ease-out;
  }
  @keyframes dlpulse {
    0% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--mp-accent) 70%, transparent); }
    100% { box-shadow: 0 0 0 12px transparent; }
  }
  .scrim {
    all: unset;
    position: fixed;
    inset: 0;
    z-index: 60;
    background: rgba(0, 0, 0, 0.25);
  }
  .drawer {
    position: fixed;
    top: 12px;
    right: 12px;
    /* 底栏播放器留在外面 */
    bottom: 92px;
    z-index: 61;
    width: min(560px, calc(100vw - 24px));
    display: flex;
    flex-direction: column;
    border-radius: 16px;
    background: var(--mp-panel);
    backdrop-filter: blur(30px);
    box-shadow: var(--mp-shadow, 0 20px 60px #0009);
    border: 1px solid var(--edge);
    animation: drawerIn 0.22s ease-out;
  }
  @keyframes drawerIn {
    from { transform: translateX(24px); opacity: 0; }
  }
  .dhead {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 14px 16px 10px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  }
  .dhead h5 {
    margin: 0;
  }
  .dhead small {
    color: var(--mp-sub);
  }
  .grow {
    flex: 1;
  }
  .dbody {
    flex: 1;
    overflow: auto;
    padding: 12px 16px;
  }
  .stack {
    display: grid;
    gap: 18px;
    min-width: 0;
  }
</style>
