<script lang="ts">
  // 自动获取设置：下载目录、整理目录、同时下载数、限速、做种策略
  import { onMount } from 'svelte';
  import Modal from '../Modal.svelte';
  import { inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { fetchApi, type FetchSettings } from '../../lib/fetch';

  let { onclose, onsaved }: { onclose: () => void; onsaved?: () => void } = $props();

  let s = $state<FetchSettings>({
    download_dir: '',
    organize_dir: '',
    concurrency: 3,
    speed_kbps: 0,
    seed_policy: 'stop',
    seed_ratio: 1,
  });
  let saving = $state(false);

  onMount(() => {
    if (!inTauri) return;
    fetchApi
      .getSettings()
      .then(v => (s = v))
      .catch(err => toast(String(err)));
  });

  async function save() {
    saving = true;
    try {
      s = await fetchApi.setSettings({ ...s, concurrency: Number(s.concurrency), speed_kbps: Number(s.speed_kbps), seed_ratio: Number(s.seed_ratio) });
      toast('设置已保存');
      onsaved?.();
      onclose();
    } catch (err) {
      toast(String(err));
    } finally {
      saving = false;
    }
  }
</script>

<Modal title="自动获取设置" {onclose}>
  <div class="fs">
    <label>
      <span>下载目录</span>
      <input class="tin" bind:value={s.download_dir} placeholder="下载\极光播放器" />
    </label>
    <label>
      <span>整理到</span>
      <input class="tin" bind:value={s.organize_dir} placeholder="视频\极光媒体库" />
    </label>
    <label>
      <span>同时下载</span>
      <input class="tin" type="number" min="1" max="8" bind:value={s.concurrency} />
    </label>
    <label>
      <span>直链限速（KB/s）</span>
      <input class="tin" type="number" min="0" bind:value={s.speed_kbps} />
    </label>
    <label>
      <span>BT 做种</span>
      <select class="sel" bind:value={s.seed_policy}>
        <option value="stop">下完即停</option>
        <option value="ratio">做种到分享率</option>
      </select>
    </label>
    {#if s.seed_policy === 'ratio'}
      <label>
        <span>分享率</span>
        <input class="tin" type="number" min="0.1" step="0.1" bind:value={s.seed_ratio} />
      </label>
    {/if}
    <div class="note">下载目录与媒体库分开：文件先下载到下载目录，识别并整理完成后才移入整理目录。可执行文件（exe、bat、msi、ps1 等）会被直接删除。限速和做种策略对新任务生效。</div>
    <div class="acts end">
      <button class="mp-btn ghost" onclick={onclose}>取消</button>
      <button class="mp-btn" onclick={save} disabled={saving}>保存</button>
    </div>
  </div>
</Modal>

<style>
  .fs {
    display: grid;
    gap: 10px;
  }
  .fs label {
    display: grid;
    grid-template-columns: 130px 1fr;
    gap: 10px;
    align-items: center;
    font-size: 13px;
  }
  .fs input,
  .fs select {
    min-width: 0;
  }
  .end {
    justify-content: flex-end;
  }
</style>
