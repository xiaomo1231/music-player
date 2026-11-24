<script lang="ts">
  // 设置页“插件”：已安装、仓库、开发者
  import { inTauri, fileUrl } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { MEDIA_TEXT } from '../../lib/fetch';
  import { plugins, pluginApi, pickPlugin, pickPluginFolder, RUNTIME_TEXT, fmtTime, type PluginView, type RepoView } from '../../lib/plugin.svelte';

  let tab = $state<'installed' | 'repos' | 'dev'>('installed');
  let list = $state.raw<PluginView[]>([]);
  let repos = $state.raw<RepoView[]>([]);
  let repoUrl = $state('');
  let auto = $state(true);
  let busy = $state(false);

  async function load() {
    if (!inTauri) return;
    try {
      list = await pluginApi.list();
      repos = await pluginApi.repos();
      auto = await pluginApi.getAutoUpdate();
    } catch (e) {
      toast(String(e));
    }
  }

  $effect(() => {
    void plugins.version;
    void load();
  });

  async function run<T>(f: () => Promise<T>, ok?: string): Promise<T | undefined> {
    if (busy) return;
    busy = true;
    try {
      const r = await f();
      if (ok) toast(ok);
      return r;
    } catch (e) {
      toast(String(e));
    } finally {
      busy = false;
    }
  }

  async function addRepo() {
    const u = repoUrl.trim();
    if (!u) return toast('先粘贴仓库的 index.json 地址');
    const r = await run(() => pluginApi.repoAdd(u), '已添加仓库');
    if (r) {
      repos = r;
      repoUrl = '';
    }
  }

  async function refresh() {
    const r = await run(() => pluginApi.repoRefresh(), '已检查更新');
    if (r) {
      repos = r;
      plugins.version++;
    }
  }

  async function removeRepo(u: string) {
    await run(() => pluginApi.repoRemove(u), '已移除仓库（已装的插件不受影响）');
    repos = repos.filter(r => r.url !== u);
  }

  async function setAuto(on: boolean) {
    auto = on;
    await run(() => pluginApi.setAutoUpdate(on));
  }

  async function loadDev() {
    if (!inTauri) return;
    const { open } = await import('@tauri-apps/plugin-dialog');
    const p = await open({ directory: true });
    if (typeof p !== 'string' || !p) return;
    const v = await run(() => pluginApi.loadDev(p));
    if (v) {
      plugins.version++;
      toast(v.problem ? `已加载「${v.name}」，但：${v.problem}` : `已加载「${v.name}」：改了文件直接生效，不用重装`);
      plugins.detail = v.id;
    }
  }

  async function packDir() {
    if (!inTauri) return;
    const { open, save } = await import('@tauri-apps/plugin-dialog');
    const dir = await open({ directory: true });
    if (typeof dir !== 'string' || !dir) return;
    const name = dir.split(/[\\/]/).pop() || 'plugin';
    const out = await save({ defaultPath: `${name}.mpx`, filters: [{ name: '插件包', extensions: ['mpx'] }] });
    if (out) await run(() => pluginApi.packDir(dir, out), '已打包');
  }

  const kinds = (p: PluginView) =>
    [...[...new Set(p.sources.flatMap(s => s.media_kinds))].map(k => MEDIA_TEXT[k] ?? k), ...(p.converters?.length ? ['格式转换'] : [])].join('、');
  const updates = $derived(list.filter(p => p.update).length);
  const STATE: Record<string, string> = { install: '安装', update: '更新', installed: '已安装', newer_app: '要先升级软件' };
</script>

<div class="card pl-card">
  <h5>插件</h5>
  <p class="note">
    插件可以接入新的下载源（音乐、视频、番剧、漫画、小说），装好后出现在“自动获取 → 我的来源”；也可以提供格式转换器，出现在“格式转换”页。双击或拖入 .mpx 文件、点“安装插件…”，或从仓库安装都可以。
    软件不自带任何插件和仓库；请只安装能合法获取内容的插件。插件不能绕过登录、验证码或 DRM 保护。
  </p>
  <div class="pl-bar">
    <div class="seg">
      <button class:on={tab === 'installed'} onclick={() => (tab = 'installed')}>已安装 {list.length || ''}</button>
      <button class:on={tab === 'repos'} onclick={() => (tab = 'repos')}>仓库 {updates ? `· ${updates} 个可更新` : ''}</button>
      <button class:on={tab === 'dev'} onclick={() => (tab = 'dev')}>开发者</button>
    </div>
    <button class="mp-btn" disabled={!inTauri} onclick={pickPlugin}>安装插件…</button>
  </div>

  {#if tab === 'installed'}
    {#if !list.length}<p class="note empty">还没有安装插件</p>{/if}
    {#each list as p (p.id)}
      <button class="pl-row" onclick={() => (plugins.detail = p.id)}>
        {#if p.icon}<img src={fileUrl(p.icon)} alt="" />{:else}<span class="pl-ico">{p.name.slice(0, 1)}</span>{/if}
        <span class="pl-main">
          <b>{p.name}</b>
          <small>v{p.version} · {RUNTIME_TEXT[p.runtime] ?? p.runtime}{kinds(p) ? ` · ${kinds(p)}` : ''}{p.dev ? ' · 开发者模式' : ''}</small>
          {#if p.problem}<small class="bad">{p.problem}</small>{:else if p.needs_settings.length}<small class="warn">先填写设置：{p.needs_settings.join('、')}</small>{/if}
        </span>
        {#if p.update}<span class="tag">可更新 v{p.update.version}</span>{/if}
        <span class="tag {p.enabled && !p.problem ? 'ok' : 'off'}">{p.problem ? '有问题' : p.enabled ? '已启用' : '已停用'}</span>
      </button>
    {/each}
  {:else if tab === 'repos'}
    <div class="frow">
      <input class="tin" style="flex:1" placeholder="粘贴仓库地址（index.json）" bind:value={repoUrl} onkeydown={e => e.key === 'Enter' && addRepo()} />
      <button class="mp-btn" disabled={busy} onclick={addRepo}>添加仓库</button>
      <button class="mp-btn ghost" disabled={busy || !repos.length} onclick={refresh}>检查更新</button>
    </div>
    <label class="frow chk"><input type="checkbox" class="mp-sw" checked={auto} onchange={e => setAuto((e.currentTarget as HTMLInputElement).checked)} /> 自动更新插件（每天检查一次；新版本要更多权限时先问你）</label>
    {#if !repos.length}<p class="note empty">还没有添加仓库。仓库由插件作者或社区提供，软件不内置任何仓库。</p>{/if}
    {#each repos as r (r.url)}
      <div class="repo">
        <div class="frow">
          <b>{r.name || r.url}</b>
          <small class="sub">{r.last_check ? `检查于 ${fmtTime(r.last_check)}` : '还没检查'}</small>
          <span class="grow"></span>
          <button class="mp-btn ghost" onclick={() => removeRepo(r.url)}>移除</button>
        </div>
        <small class="sub">{r.url}</small>
        {#if r.last_error}<div class="bad">{r.last_error}</div>{/if}
        {#each r.plugins as e (e.id)}
          <div class="pl-entry">
            <span class="pl-main">
              <b>{e.name}</b>
              <small>v{e.version} · {RUNTIME_TEXT[e.runtime] ?? e.runtime}{e.media_kinds.length ? ` · ${e.media_kinds.map(k => MEDIA_TEXT[k] ?? k).join('、')}` : ''}{e.installed_version ? ` · 已装 v${e.installed_version}` : ''}</small>
              {#if e.description}<small>{e.description}</small>{/if}
            </span>
            {#if e.state === 'install' || e.state === 'update'}
              <button class="mp-btn" onclick={() => (plugins.pendingRepo = { repo: r.url, id: e.id })}>{STATE[e.state]}</button>
            {:else}
              <span class="tag">{STATE[e.state] ?? e.state}</span>
            {/if}
          </div>
        {/each}
      </div>
    {/each}
  {:else}
    <p class="note">写插件时用：直接加载插件文件夹，改了文件下次调用就生效，不用重新打包安装。开发文档见项目里的 docs/插件开发.md。</p>
    <div class="frow">
      <button class="mp-btn" disabled={!inTauri || busy} onclick={loadDev}>加载插件文件夹…</button>
      <button class="mp-btn ghost" disabled={!inTauri || busy} onclick={pickPluginFolder}>从文件夹安装…</button>
      <button class="mp-btn ghost" disabled={!inTauri || busy} onclick={packDir}>把文件夹打包成 .mpx…</button>
    </div>
    <p class="note">每个插件的“测试”和“日志”在插件详情里。</p>
  {/if}
</div>

<style>
  .note {
    color: var(--mp-sub);
    font-size: 12.5px;
    line-height: 1.6;
  }
  .empty {
    padding: 10px 0;
  }
  .pl-bar,
  .frow {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
    margin: 8px 0;
  }
  .chk {
    font-size: 13px;
    cursor: pointer;
  }
  .pl-bar {
    justify-content: space-between;
  }
  .pl-row,
  .pl-entry {
    display: flex;
    gap: 10px;
    align-items: center;
    width: 100%;
    text-align: left;
    padding: 8px 6px;
    border-radius: 8px;
    background: none;
    border: 0;
    color: inherit;
    font: inherit;
  }
  .pl-row {
    cursor: pointer;
  }
  .pl-row:hover {
    background: rgba(127, 127, 127, 0.08);
  }
  .pl-row img,
  .pl-ico {
    width: 32px;
    height: 32px;
    border-radius: 8px;
    flex-shrink: 0;
    object-fit: cover;
  }
  .pl-ico {
    display: grid;
    place-items: center;
    background: rgba(127, 127, 127, 0.15);
    font-weight: 600;
  }
  .pl-main {
    display: grid;
    flex: 1;
    min-width: 0;
    gap: 2px;
  }
  .pl-main small,
  .sub {
    color: var(--mp-sub);
    font-size: 12px;
  }
  .bad {
    color: #ff8a8a !important;
    font-size: 12.5px;
  }
  .warn {
    color: #ffb45a !important;
  }
  .tag {
    font-size: 11px;
    padding: 2px 7px;
    border-radius: 6px;
    background: rgba(127, 127, 127, 0.15);
    white-space: nowrap;
  }
  .tag.ok {
    background: rgba(80, 200, 120, 0.16);
    color: #6fd896;
  }
  .tag.off {
    color: var(--mp-sub);
  }
  .repo {
    border-top: 1px solid rgba(127, 127, 127, 0.15);
    padding: 8px 0;
  }
  .grow {
    flex: 1;
  }
</style>
