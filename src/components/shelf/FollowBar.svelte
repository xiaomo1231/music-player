<script lang="ts">
  // 作品详情页里的追更条：追更中 · 来源 · 上次检查；“检查更新”有新内容就直接加入下载
  import { onMount } from 'svelte';
  import { follow, agoText, UNIT } from '../../lib/follow.svelte';

  let { kind, name }: { kind: 'manga' | 'novel' | 'anime'; name: string } = $props();

  onMount(() => void follow.load(true));
  const subs = $derived(follow.of(kind, name));
</script>

{#each subs as s (s.id)}
  <div class="fb">
    <span class="badge2 ok">追更中</span>
    <small>{s.source_name || '来源已删除'} · {follow.result[s.id] ?? agoText(s.last_check)}</small>
    <button class="mp-btn ghost sm" disabled={follow.checking[s.id]} onclick={() => follow.check(s, UNIT[kind])}>
      {follow.checking[s.id] ? '正在检查…' : '检查更新'}
    </button>
    <button class="mp-btn ghost sm" onclick={() => follow.cancel(s)}>取消追更</button>
  </div>
{/each}

<style>
  .fb {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
    margin-top: 10px;
    font-size: 12.5px;
  }
  .fb small {
    color: var(--mp-sub);
  }
  .sm {
    padding: 4px 10px;
    font-size: 12px;
  }
</style>
