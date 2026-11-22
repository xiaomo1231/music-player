<script lang="ts">
  // 桌面歌词窗口的内容：只显示当前一行歌词，大字带描边。窗口由 shell_desktop_lyrics 创建（index.html#lyrics）
  import { onMount } from 'svelte';
  import { emit } from '@tauri-apps/api/event';
  import { inTauri, listen } from '../../lib/api';

  let text = $state('');

  onMount(() => {
    document.documentElement.style.background = 'transparent';
    document.body.style.background = 'transparent';
    if (!inTauri) {
      text = '桌面歌词预览';
      return;
    }
    let off: (() => void) | undefined;
    listen<string>('lyrics://line', t => (text = t)).then(f => (off = f));
    // 主窗口收到请求后会立即推送当前这一行
    emit('lyrics://ask').catch(() => {});
    return () => off?.();
  });
</script>

<div class="dl">
  {#if text}
    {@const [main, sub] = text.split('\n')}
    <p>{main}</p>
    {#if sub}<p class="sub">{sub}</p>{/if}
  {/if}
</div>

<style>
  .dl {
    position: fixed;
    inset: 0;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 2px;
    padding: 0 24px;
    background: transparent;
    pointer-events: none;
    user-select: none;
  }
  p {
    margin: 0;
    max-width: 100%;
    text-align: center;
    font-size: clamp(22px, 4.2vh, 46px);
    font-weight: 800;
    line-height: 1.35;
    color: #fff;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    -webkit-text-stroke: 2px rgba(0, 0, 0, 0.75);
    paint-order: stroke fill;
    text-shadow: 0 2px 14px rgba(0, 0, 0, 0.6);
  }
  p.sub {
    font-size: clamp(15px, 2.6vh, 28px);
    font-weight: 700;
    -webkit-text-stroke-width: 1.5px;
    opacity: 0.9;
  }
</style>
