<script lang="ts">
  import Modal from './Modal.svelte';
  import { ui, toast } from '../lib/ui.svelte';
  const o = ui.oled;
  const save = () => ui.saveOled();
  function refresh() {
    ui.close();
    ui.refreshRun = 0;
    requestAnimationFrame(() => {
      ui.refreshRun = 1;
      setTimeout(() => (ui.refreshRun = 0), 6100);
    });
  }
</script>

<Modal title="OLED 防烧屏" onclose={() => ui.close()}>
  <p class="mp-desc">为长时间挂在桌面当背景而设计。每一项都可以单独开关，设置会自动保存。</p>
  <label class="mp-opt"><div>纯黑背景<small>背景切换为 #000，OLED 像素完全熄灭，省电并减少老化</small></div><input type="checkbox" class="mp-sw" bind:checked={o.black} onchange={save} /></label>
  <label class="mp-opt"><div>像素位移<small>整个画面每隔一段时间整体偏移几个像素，肉眼几乎无感</small></div>
    <select bind:value={o.shiftSec} onchange={() => { save(); toast('像素位移间隔：' + o.shiftSec + ' 秒'); }}>
      <option value={30}>30 秒</option><option value={60}>1 分钟</option><option value={180}>3 分钟</option>
    </select>
    <input type="checkbox" class="mp-sw" bind:checked={o.shift} onchange={save} /></label>
  <label class="mp-opt"><div>元素缓慢漂移<small>唱片、歌词等大块元素沿不规则轨迹缓慢游走，避免固定位置残影</small></div><input type="checkbox" class="mp-sw" bind:checked={o.drift} onchange={save} /></label>
  <label class="mp-opt"><div>无操作自动变暗<small>鼠标键盘静止后逐渐降低亮度，动一下立即恢复</small></div>
    <select bind:value={o.dimSec} onchange={save}><option value={60}>1 分钟</option><option value={300}>5 分钟</option><option value={900}>15 分钟</option></select>
    <select bind:value={o.dimLv} onchange={save}><option value={0.4}>轻度</option><option value={0.65}>中度</option><option value={0.85}>重度</option></select>
    <input type="checkbox" class="mp-sw" bind:checked={o.dim} onchange={save} /></label>
  <label class="mp-opt"><div>自动隐藏静态控件<small>沉浸模式下按钮、进度条在几秒后淡出，只留会动的内容</small></div><input type="checkbox" class="mp-sw" bind:checked={o.hide} onchange={save} /></label>
  <label class="mp-opt"><div>压低静态元素亮度<small>侧边栏、图标等长期不动的元素降到一半亮度，悬停时恢复</small></div><input type="checkbox" class="mp-sw" bind:checked={o.stat} onchange={save} /></label>
  <div class="mp-opt"><div>像素刷新<small>全屏轮流显示红绿蓝白黑约 6 秒，让所有子像素均匀工作一次</small></div><button class="mp-btn ghost" onclick={refresh}>立即运行</button></div>
</Modal>
