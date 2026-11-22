<script lang="ts">
  import Modal from './Modal.svelte';
  import { ui } from '../lib/ui.svelte';
  import { COMIC_KEYS, COMIC_MOUSE, NOVEL_KEYS } from '../lib/readerprefs';
  const music: [string, string][] = [
    ['空格', '播放 / 暂停'], ['Ctrl + ← / →', '上一首 / 下一首'], ['← / →', '后退 / 前进 5 秒'], ['↑ / ↓', '音量'], ['M', '静音'],
    ['Ctrl + D', '收藏当前歌曲'], ['Ctrl + Shift + D', '加入歌单'], ['Ctrl + H / Ctrl + R', '随机 / 循环模式'],
    ['F', '沉浸模式'], ['Ctrl + M', '迷你播放器'], ['/ 或 Ctrl + F', '搜索'], ['Q', '播放队列'], ['E', '均衡器'], ['O', 'OLED 防烧屏'], ['?', '快捷键列表'],
  ];
  const multi: [string, string][] = [
    ['Ctrl + 单击', '点选 / 取消一项'], ['Shift + 单击', '连选'], ['Ctrl + A', '全选当前列表'], ['Esc', '取消多选'],
    ['右键选中的项', '批量菜单'], ['拖动选中的歌', '歌单里多首一起调整顺序'],
  ];
  const global: [string, string][] = [
    ['多媒体键', '播放 / 暂停、上一首、下一首'], ['Ctrl + Alt + ← / →', '上一首 / 下一首'], ['Ctrl + Alt + ↑ / ↓', '音量'],
    ['Ctrl + Alt + P', '显示 / 隐藏主窗口'], ['Ctrl + Alt + D', '收藏当前歌曲'],
  ];
</script>

<Modal title="快捷键" onclose={() => ui.close()} wide>
  <p class="mp-desc">视频播放时、漫画和小说阅读器里按 ? 可以看到各自的快捷键。全局热键在软件最小化时也有效，可以在设置里关闭。</p>
  <div class="kv">{#each music as [k, v]}<span><kbd>{k}</kbd></span><span>{v}</span>{/each}</div>
  <h4 style="margin:16px 0 8px">列表多选</h4>
  <p class="mp-desc">也可以把鼠标移到序号（卡片左上角）上点勾选框，或点页面上的“多选”按钮。多选时单击是勾选，双击仍然播放。</p>
  <div class="kv">{#each multi as [k, v]}<span><kbd>{k}</kbd></span><span>{v}</span>{/each}</div>
  <h4 style="margin:16px 0 8px">漫画阅读器</h4>
  <div class="kv">{#each COMIC_KEYS as [k, v]}<span><kbd>{k}</kbd></span><span>{v}</span>{/each}</div>
  <h4 style="margin:16px 0 8px">漫画阅读器 · 鼠标 / 触摸板</h4>
  <div class="kv">{#each COMIC_MOUSE as [k, v]}<span><kbd>{k}</kbd></span><span>{v}</span>{/each}</div>
  <h4 style="margin:16px 0 8px">小说阅读器</h4>
  <div class="kv">{#each NOVEL_KEYS as [k, v]}<span><kbd>{k}</kbd></span><span>{v}</span>{/each}</div>
  <h4 style="margin:16px 0 8px">全局热键</h4>
  <div class="kv">{#each global as [k, v]}<span><kbd>{k}</kbd></span><span>{v}</span>{/each}</div>
</Modal>
