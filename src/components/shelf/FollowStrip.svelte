<script lang="ts">
  // 书架顶部：已经追更、但书架上还没有这部作品（一话都还没下）的作品卡
  import { onMount } from 'svelte';
  import { follow, agoText, UNIT } from '../../lib/follow.svelte';
  import { ui } from '../../lib/ui.svelte';

  let { kind, names }: { kind: 'manga' | 'novel' | 'anime'; names: string[] } = $props();

  onMount(() => void follow.load(true));
  const list = $derived(follow.orphans(kind, names));
</script>

{#if list.length}
  <div class="fs">
    <div class="fs-head">
      <b>追更中</b>
      <small>还没有下载过的作品；订阅前已经出的{UNIT[kind]}可以去“自动获取”里选{UNIT[kind]}下载</small>
      <span class="grow"></span>
      <button class="mp-chip" onclick={() => ui.go('get')}>去自动获取</button>
    </div>
    <div class="fs-grid">
      {#each list as s (s.id)}
        <div class="fs-card">
          {#if s.cover}
            <img src={s.cover} alt="" loading="lazy" referrerpolicy="no-referrer" />
          {:else}
            <div class="ph">{s.query.slice(0, 2)}</div>
          {/if}
          <div class="fs-body">
            <b title={s.query}>{s.query}</b>
            <small>{s.source_name || '来源已删除'} · {follow.result[s.id] ?? agoText(s.last_check)}</small>
            <div class="fs-acts">
              <button class="mp-btn ghost sm" disabled={follow.checking[s.id]} onclick={() => follow.check(s, UNIT[kind])}>
                {follow.checking[s.id] ? '正在检查…' : '检查更新'}
              </button>
              <button class="mp-btn ghost sm" onclick={() => follow.cancel(s)}>取消追更</button>
            </div>
          </div>
        </div>
      {/each}
    </div>
  </div>
{/if}

<style>
  .fs {
    margin: 14px 0 6px;
  }
  .fs-head {
    display: flex;
    gap: 8px;
    align-items: baseline;
    flex-wrap: wrap;
    margin-bottom: 8px;
  }
  .fs-head small,
  .fs-body small {
    color: var(--mp-sub);
    font-size: 12px;
  }
  .grow {
    flex: 1;
  }
  .fs-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 8px;
  }
  .fs-card {
    display: flex;
    gap: 10px;
    padding: 10px;
    border-radius: 12px;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.06);
    min-width: 0;
  }
  .fs-card img,
  .ph {
    width: 48px;
    height: 66px;
    border-radius: 8px;
    object-fit: cover;
    flex: none;
  }
  .ph {
    display: grid;
    place-items: center;
    background: rgba(255, 255, 255, 0.08);
    color: var(--mp-sub);
    font-size: 12px;
  }
  .fs-body {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }
  .fs-body b {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }
  .fs-acts {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: auto;
  }
  .sm {
    padding: 4px 10px;
    font-size: 12px;
  }
</style>
