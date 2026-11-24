<script lang="ts">
  // 设置页“封面与资料”：联网获取番剧 / 电视剧 / 电影的官方海报（后端 artmeta.rs，设置键 artwork_online）
  import { invoke, inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { loadArt } from '../../lib/media.svelte';

  let cfg = $state({ enabled: true, tmdb_key: '', proxy: '' });
  let testing = $state(false);
  let result = $state<string[]>([]);

  if (inTauri) {
    invoke<Partial<typeof cfg> | null>('get_setting', { key: 'artwork_online' })
      .then(v => (cfg = { ...cfg, ...(v ?? {}) }))
      .catch(() => {});
  }

  async function save(patch: Partial<typeof cfg> = {}) {
    cfg = { ...cfg, ...patch };
    if (!inTauri) return;
    await invoke('set_setting', { key: 'artwork_online', value: $state.snapshot(cfg) }).catch(() => {});
  }

  async function test() {
    await save();
    testing = true;
    try {
      result = await invoke<string[]>('art_test');
    } catch (e) {
      result = [String(e)];
    } finally {
      testing = false;
    }
  }

  async function refreshAll() {
    await save();
    toast('正在后台重新匹配，手动选过的封面会保留');
    try {
      await invoke('art_refresh_all');
      await loadArt();
      toast('重新匹配完成');
    } catch (e) {
      toast(String(e));
    }
  }
</script>

<div class="card">
  <h5>封面与资料</h5>
  <label class="mp-opt">
    <div>联网获取官方海报<small>番剧和动画电影查 Bangumi，电视剧和电影查 TMDB。只发送作品名，不发送文件路径；海报缓存在本机</small></div>
    <input type="checkbox" class="mp-sw" checked={cfg.enabled} onchange={e => save({ enabled: (e.currentTarget as HTMLInputElement).checked })} />
  </label>
  <div class="mp-opt art-in">
    <div>TMDB API Key<small>不填也能用，只是电视剧和真人电影不会联网找海报。在 themoviedb.org 的“设置 → API”里申请</small></div>
    <input class="tin" type="password" placeholder="API 密钥（v3）或读访问令牌" bind:value={cfg.tmdb_key} onchange={() => save()} autocomplete="off" />
  </div>
  <div class="mp-opt art-in">
    <div>TMDB 代理<small>国内常连不上 TMDB 时填，例如 http://127.0.0.1:7890；Bangumi 不走代理</small></div>
    <input class="tin" placeholder="留空表示直连" bind:value={cfg.proxy} onchange={() => save()} />
  </div>
  <div class="rowbtns">
    <button class="mp-btn ghost" disabled={!inTauri || testing} onclick={test}>{testing ? '测试中…' : '测试连接'}</button>
    <button class="mp-btn ghost" disabled={!inTauri || !cfg.enabled} onclick={refreshAll}>全部重新匹配</button>
  </div>
  {#if result.length}
    <ul class="note art-res">
      {#each result as r}<li>{r}</li>{/each}
    </ul>
  {/if}
  <p class="note">匹配不准的作品可以在卡片右键菜单里“更换封面”。资料来源：Bangumi 番组计划（bgm.tv）。本产品使用 TMDB API，但未经 TMDB 认可或认证。</p>
</div>

<style>
  .art-in { flex-wrap: wrap; gap: 8px; }
  .art-in > div { flex: 1 1 100%; }
  .art-in .tin { flex: 1 1 260px; }
  .art-res { margin: 10px 0 0; padding-left: 18px; }
</style>
