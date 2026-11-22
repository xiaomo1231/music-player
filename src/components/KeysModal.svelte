<script lang="ts">
  import Modal from './Modal.svelte';
  import { ui } from '../lib/ui.svelte';
  const music: [string, string][] = [
    ['空格', '播放 / 暂停'], ['Ctrl + ← / →', '上一首 / 下一首'], ['← / →', '后退 / 前进 5 秒'], ['↑ / ↓', '音量'], ['M', '静音'],
    ['Ctrl + D', '收藏当前歌曲'], ['Ctrl + Shift + D', '加入歌单'], ['Ctrl + H / Ctrl + R', '随机 / 循环模式'],
    ['F', '沉浸模式'], ['Ctrl + M', '迷你播放器'], ['/ 或 Ctrl + F', '搜索'], ['Q', '播放队列'], ['E', '均衡器'], ['O', 'OLED 防烧屏'], ['?', '快捷键列表'],
  ];
  const global: [string, string][] = [
    ['多媒体键', '播放 / 暂停、上一首、下一首'], ['Ctrl + Alt + ← / →', '上一首 / 下一首'], ['Ctrl + Alt + ↑ / ↓', '音量'],
    ['Ctrl + Alt + P', '显示 / 隐藏主窗口'], ['Ctrl + Alt + D', '收藏当前歌曲'],
  ];
</script>

<Modal title="快捷键" onclose={() => ui.close()} wide>
  <p class="mp-desc">视频播放时按 ? 可以看到视频专用的快捷键。全局热键在软件最小化时也有效，可以在设置里关闭。</p>
  <div class="kv">{#each music as [k, v]}<span><kbd>{k}</kbd></span><span>{v}</span>{/each}</div>
  <h4 style="margin:16px 0 8px">全局热键</h4>
  <div class="kv">{#each global as [k, v]}<span><kbd>{k}</kbd></span><span>{v}</span>{/each}</div>
</Modal>
