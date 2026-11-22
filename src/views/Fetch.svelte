<script lang="ts">
  // 自动获取页：左边“我的来源”，右边“搜索并下载”和“下载列表”；设置放在弹窗里
  import { onMount } from 'svelte';
  import Icon from '../lib/Icon.svelte';
  import { listen, inTauri } from '../lib/api';
  import { toast } from '../lib/ui.svelte';
  import { fetchApi, type TaskRow } from '../lib/fetch';
  import Sources from '../components/fetch/Sources.svelte';
  import Search from '../components/fetch/Search.svelte';
  import Downloads from '../components/fetch/Downloads.svelte';
  import FetchSettings from '../components/fetch/FetchSettings.svelte';
  import Bili from '../components/fetch/Bili.svelte';

  let tasks = $state<TaskRow[]>([]);
  let settingsOpen = $state(false);

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
      <button class="mp-btn ghost" onclick={() => (settingsOpen = true)}><Icon name="gear" /> 设置</button>
    </div>
  </header>

  <div class="get" style="margin-top:14px">
    <div class="card">
      <Sources onchange={loadTasks} />
    </div>
    <div class="stack">
      <div class="card">
        <Bili onqueued={loadTasks} />
      </div>
      <div class="card">
        <Search onqueued={loadTasks} />
      </div>
      <div class="card">
        <h5>下载列表 <span class="badge2">{tasks.length}</span></h5>
        <Downloads {tasks} onchange={loadTasks} />
      </div>
    </div>
  </div>
</section>

{#if settingsOpen}
  <FetchSettings onclose={() => (settingsOpen = false)} onsaved={loadTasks} />
{/if}

<style>
  .stack {
    display: grid;
    gap: 18px;
    min-width: 0;
  }
</style>
