<script lang="ts">
  // 插件详情：设置、下载源状态、权限、测试、日志、启用 / 停用、更新、回滚、卸载
  import Modal from '../Modal.svelte';
  import { inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { confirmDlg } from '../../lib/items.svelte';
  import { MEDIA_TEXT } from '../../lib/fetch';
  import {
    plugins,
    pluginApi,
    RUNTIME_TEXT,
    TOOL_TEXT,
    CODE_TEXT,
    splitCode,
    fmtTime,
    type PluginView,
    type SettingsView,
    type SettingDef,
    type LogLine,
    type Stage,
  } from '../../lib/plugin.svelte';

  const id = $derived(plugins.detail);
  let v = $state.raw<PluginView | null>(null);
  let sv = $state.raw<SettingsView | null>(null);
  let values = $state<Record<string, unknown>>({});
  /** 密钥：新填的值；null 表示清除 */
  let secrets = $state<Record<string, string | null>>({});
  let tab = $state<'main' | 'test' | 'logs'>('main');
  let logs = $state.raw<LogLine[]>([]);
  let stages = $state.raw<Stage[]>([]);
  let testSource = $state('');
  let testQuery = $state('');
  let testKind = $state('');
  let busy = $state(false);

  async function load() {
    const cur = id;
    if (!cur) return;
    try {
      const all = await pluginApi.list();
      if (cur !== id) return;
      v = all.find(p => p.id === cur) ?? null;
      if (!v) return;
      if (!testSource) testSource = v.sources[0]?.id ?? '';
      sv = v.has_settings ? await pluginApi.getSettings(cur) : null;
      values = sv ? { ...sv.values } : {};
      secrets = {};
    } catch (e) {
      toast(String(e));
    }
  }

  $effect(() => {
    void plugins.version;
    void load();
  });

  $effect(() => {
    if (tab === 'logs' && id) void loadLogs();
  });

  async function loadLogs() {
    try {
      logs = (await pluginApi.logs(id)).slice().reverse();
    } catch (e) {
      toast(String(e));
    }
  }

  function close() {
    plugins.detail = '';
    v = null;
    sv = null;
    tab = 'main';
    stages = [];
    testSource = '';
  }

  async function run(f: () => Promise<unknown>, ok?: string) {
    if (busy) return;
    busy = true;
    try {
      await f();
      plugins.version++;
      if (ok) toast(ok);
    } catch (e) {
      toast(String(e));
    } finally {
      busy = false;
    }
  }

  /** 下载源上次出错：重新连一次，看是不是好了 */
  const recheck = (source: string) =>
    run(async () => {
      const err = await pluginApi.recheck(id, source);
      const e = splitCode(err);
      toast(err ? `还是不行（${CODE_TEXT[e.code]?.label ?? '出错'}）${e.text}` : '已恢复正常');
    });

  const save = () =>
    run(async () => {
      const out: Record<string, unknown> = { ...values };
      for (const [k, s] of Object.entries(secrets)) if (s === null || s) out[k] = s;
      await pluginApi.saveSettings(id, out);
    }, '设置已保存');

  async function pickPath(s: SettingDef) {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const p = await open({ directory: s.type === 'folder', multiple: false });
    if (typeof p === 'string' && p) values[s.key] = p;
  }

  const optValue = (o: SettingDef['options'][number]) => (typeof o === 'string' ? o : o.value);
  const optLabel = (o: SettingDef['options'][number]) => (typeof o === 'string' ? o : (o.label ?? o.value));

  async function uninstall() {
    if (!v) return;
    const ok = await confirmDlg('卸载插件', `卸载「${v.name}」：移除它提供的下载源和规则。已经下载的文件不受影响。`, { ok: '卸载', danger: true });
    if (!ok) return;
    const purge = await confirmDlg('同时删除设置？', '要不要连插件的设置、保存的密钥和缓存一起删掉？点“取消”会保留，以后重装可以直接用。', { ok: '一起删掉' });
    await run(() => pluginApi.uninstall(id, purge), '已卸载');
    close();
  }

  /** 文件夹已被手动删掉：不用再确认，直接去掉记录（保留设置） */
  async function removeMissing() {
    if (!v) return;
    const name = v.name;
    await run(() => pluginApi.uninstall(id, false), `已移除「${name}」`);
    close();
  }

  async function test() {
    if (!v || busy) return;
    busy = true;
    stages = [];
    try {
      stages = await pluginApi.test(id, testSource, testQuery, testKind);
    } catch (e) {
      toast(String(e));
    } finally {
      busy = false;
    }
  }

  async function exportPkg() {
    if (!v) return;
    const { save } = await import('@tauri-apps/plugin-dialog');
    const p = await save({ defaultPath: `${v.id}-${v.version}.mpx`, filters: [{ name: '插件包', extensions: ['mpx'] }] });
    if (p) await run(() => pluginApi.pack(id, p), '已导出插件包');
  }

  const kinds = (ks: string[]) => (ks.length ? ks.map(k => MEDIA_TEXT[k] ?? k).join('、') : '全部类型');
  const testKinds = $derived(v?.sources.find(s => s.id === testSource)?.media_kinds ?? []);
</script>

{#if id}
  <Modal title={v ? v.name : '插件'} onclose={close} wide>
    {#if !v}
      <p class="note">正在读取…</p>
    {:else}
      <div class="pd-head">
        <span class="badge2">{RUNTIME_TEXT[v.runtime] ?? v.runtime}</span>
        <small>v{v.version}{v.author ? ` · ${v.author}` : ''}{v.dev ? ' · 开发者模式' : ''}</small>
        <span class="grow"></span>
        {#if !v.missing}
        <label class="sw" title={v.enabled ? '停用' : '启用'}>
          <input type="checkbox" class="mp-sw" checked={v.enabled} disabled={busy} onchange={() => run(() => pluginApi.setEnabled(id, !v!.enabled))} />
          {v.enabled ? '已启用' : '已停用'}
        </label>
        {/if}
      </div>
      {#if v.description}<p class="note">{v.description}</p>{/if}
      {#if v.missing}
        <div class="pd-missing">
          <span>插件文件夹不见了（可能被手动删除），已经停用。把它从列表里移除就行；设置会保留，重新安装后还能用。</span>
          <button class="mp-btn ghost danger" disabled={busy} onclick={removeMissing}>移除</button>
        </div>
      {:else if v.problem}<p class="pd-err">{v.problem}</p>{/if}
      {#if v.needs_settings.length}<p class="pd-warn">先填写：{v.needs_settings.join('、')}</p>{/if}
      {#if v.update}
        <div class="pd-upd">
          有新版本 v{v.update.version}
          {#if v.update.new_permissions.length}
            （多要权限：{v.update.new_permissions.join('；')}）
            <button class="mp-btn" onclick={() => (plugins.pendingRepo = { repo: v!.update!.repo, id })}>查看并更新</button>
          {:else}
            <button class="mp-btn" disabled={busy} onclick={() => run(() => pluginApi.update(id), '已更新')}>更新</button>
          {/if}
        </div>
      {/if}

      <div class="seg pd-tabs">
        <button class:on={tab === 'main'} onclick={() => (tab = 'main')}>概况与设置</button>
        <button class:on={tab === 'test'} onclick={() => (tab = 'test')}>测试</button>
        <button class:on={tab === 'logs'} onclick={() => (tab = 'logs')}>日志</button>
      </div>

      {#if tab === 'main'}
        {#if v.sources.length}
          <h6>下载源</h6>
          {#each v.sources as s (s.id)}
            <div class="pd-src">
              <b>{s.name}</b>
              <small>{kinds(s.media_kinds)} · {s.capabilities.join(' / ')}</small>
              {#if s.status}
                {#if s.status.code}
                  <span class="st bad" title={s.status.message}>{CODE_TEXT[s.status.code]?.label ?? s.status.code}</span>
                  <button class="mp-btn ghost" title="重新连接一次，看看是不是好了" disabled={busy} onclick={() => recheck(s.id)}>{busy ? '检查中…' : '重试'}</button>
                {:else}
                  <span class="st ok">{s.status.method} 正常</span>
                {/if}
              {/if}
            </div>
            {#if s.status?.code}<div class="note">{s.status.message}{CODE_TEXT[s.status.code] ? ` ${CODE_TEXT[s.status.code].hint}` : ''}</div>{/if}
          {/each}
          <p class="note">在“自动获取 → 我的来源”里可以单独开关每个下载源。</p>
        {/if}

        {#if v.converters?.length}
          <h6>格式转换</h6>
          {#each v.converters as c (c.id)}
            <div class="pd-src">
              <b>{c.name}</b>
              <small>{c.inputs.join(' / ')} → {c.outputs.map(o => o.name).join(' / ')}{c.exec ? ' · 命令行程序' : ''}</small>
            </div>
            {#if c.note}<div class="note">{c.note}</div>{/if}
          {/each}
          <p class="note">在“格式转换”页的“插件”一行里使用。</p>
        {/if}

        {#if sv && sv.schema.length}
          <h6>设置</h6>
          <div class="pd-form">
            {#each sv.schema as s (s.key)}
              <label class="pd-field">
                <span>{s.label || s.key}{s.required ? ' *' : ''}</span>
                {#if s.type === 'bool'}
                  <input type="checkbox" class="mp-sw" checked={!!values[s.key]} onchange={e => (values[s.key] = (e.currentTarget as HTMLInputElement).checked)} />
                {:else if s.type === 'select'}
                  <select class="sel" value={String(values[s.key] ?? '')} onchange={e => (values[s.key] = (e.currentTarget as HTMLSelectElement).value)}>
                    {#each s.options as o (optValue(o))}<option value={optValue(o)}>{optLabel(o)}</option>{/each}
                  </select>
                {:else if s.type === 'int'}
                  <input class="tin" type="number" value={values[s.key] ?? ''} oninput={e => (values[s.key] = (e.currentTarget as HTMLInputElement).value)} />
                {:else if s.type === 'secret'}
                  <span class="row">
                    <input
                      class="tin"
                      type="password"
                      autocomplete="off"
                      placeholder={sv.secrets_set.includes(s.key) && secrets[s.key] !== null ? '已保存（留空不变）' : '未填写'}
                      value={secrets[s.key] ?? ''}
                      oninput={e => (secrets[s.key] = (e.currentTarget as HTMLInputElement).value)}
                    />
                    {#if sv.secrets_set.includes(s.key)}<button class="mp-btn ghost" onclick={() => (secrets[s.key] = null)}>清除</button>{/if}
                  </span>
                {:else}
                  <span class="row">
                    <input class="tin" value={String(values[s.key] ?? '')} oninput={e => (values[s.key] = (e.currentTarget as HTMLInputElement).value)} />
                    {#if (s.type === 'folder' || s.type === 'file') && inTauri}<button class="mp-btn ghost" onclick={() => pickPath(s)}>选择…</button>{/if}
                  </span>
                {/if}
                {#if s.help}<small>{s.help}</small>{/if}
              </label>
            {/each}
          </div>
          <p class="note">密钥保存在系统的凭据管理器里，不会写进设置文件。</p>
          <div class="rowbtns"><button class="mp-btn" disabled={busy} onclick={save}>保存设置</button></div>
        {/if}

        <h6>权限</h6>
        <ul class="pd-perm">
          <li>运行方式：{v.trust === 'full' ? '程序（在你的电脑上直接运行，不受沙箱限制）' : '沙箱（只能访问声明的网站，不能读写文件）'}</li>
          <li>网站：{v.hosts.length ? v.hosts.join('、') : '不联网'}</li>
          {#if v.tools.length}<li>工具：{v.tools.map(t => TOOL_TEXT[t] ?? t).join('、')}</li>{/if}
          {#if v.login !== 'none'}
            <li class="row">
              使用“网站登录”里保存的登录：
              <input type="checkbox" class="mp-sw" checked={v.login_granted} disabled={busy} onchange={() => run(() => pluginApi.setLogin(id, !v!.login_granted))} />
              {v.login === 'required' && !v.login_granted ? '（这个插件需要登录才能用）' : ''}
            </li>
          {/if}
        </ul>

        <h6>其他</h6>
        <div class="pd-kv">
          <div><span>id</span>{v.id}</div>
          <div><span>来源</span>{v.dev ? `开发者模式：${v.dir}` : v.origin === 'local' ? '本地安装' : v.origin}</div>
          {#if v.installed_at}<div><span>安装于</span>{fmtTime(v.installed_at)}{v.updated_at !== v.installed_at ? `，更新于 ${fmtTime(v.updated_at)}` : ''}</div>{/if}
          {#if v.sha256}<div><span>摘要</span><code>{v.sha256.slice(0, 16)}</code></div>{/if}
          {#if v.homepage}<div><span>主页</span>{v.homepage}</div>{/if}
          {#if v.license}<div><span>许可</span>{v.license}</div>{/if}
        </div>
        {#if v.readme}
          <details class="pd-readme"><summary>说明文档</summary><pre>{v.readme}</pre></details>
        {/if}

        <div class="rowbtns pd-actions">
          {#if !v.missing}
            <button class="mp-btn ghost" disabled={!inTauri} onclick={() => run(() => pluginApi.openDir(id))}>打开文件夹</button>
            <button class="mp-btn ghost" disabled={!inTauri || busy} onclick={exportPkg}>导出插件包</button>
          {/if}
          {#if v.has_prev}<button class="mp-btn ghost" disabled={busy} onclick={() => run(() => pluginApi.rollback(id), '已回到上一个版本')}>回到上一版本</button>{/if}
          <button class="mp-btn ghost danger" disabled={busy} onclick={uninstall}>卸载</button>
        </div>
      {:else if tab === 'test' && !v.sources.length}
        <p class="note">这个插件没有下载源。格式转换器请直接在“格式转换”页拖入文件试转，过程和出错信息记在“日志”里。</p>
      {:else if tab === 'test'}
        <p class="note">自检、搜索一次、解析第一条结果的下载地址（不会真的下载）。每一步的用时、错误码和返回内容都会列出来。</p>
        <div class="frow">
          {#if v.sources.length > 1}
            <select class="sel" bind:value={testSource}>
              {#each v.sources as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
            </select>
          {/if}
          {#if testKinds.length > 1}
            <select class="sel" bind:value={testKind}>
              {#each testKinds as k (k)}<option value={k}>{MEDIA_TEXT[k] ?? k}</option>{/each}
            </select>
          {/if}
          <input class="tin" style="flex:1" placeholder="搜索词（留空只做自检）" bind:value={testQuery} onkeydown={e => e.key === 'Enter' && test()} />
          <button class="mp-btn" disabled={busy} onclick={test}>{busy ? '正在测试…' : '测试'}</button>
        </div>
        {#each stages as s, k (k)}
          <div class="pd-stage" class:bad={!s.ok}>
            <div class="row">
              <b>{s.ok ? '✓' : '✕'} {s.name}</b>
              <small>{s.ms} 毫秒</small>
              {#if s.code}<span class="st bad">{CODE_TEXT[s.code]?.label ?? s.code}</span>{/if}
            </div>
            {#if s.error}<div class="pd-err">{s.error}</div>{/if}
            {#if s.summary}<div class="note">{s.summary}</div>{/if}
            {#if s.preview !== null && s.preview !== undefined && s.preview !== ''}
              <details><summary>返回内容</summary><pre>{JSON.stringify(s.preview, null, 2)}</pre></details>
            {/if}
          </div>
        {/each}
      {:else}
        <div class="frow">
          <button class="mp-btn ghost" onclick={loadLogs}>刷新</button>
          <button class="mp-btn ghost" onclick={() => run(async () => { await pluginApi.clearLogs(id); logs = []; })}>清空</button>
        </div>
        {#if !logs.length}<p class="note">还没有日志。搜索或下载一次后再来看。</p>{/if}
        <div class="logs">
          {#each logs as l, k (k)}
            <div class="log {l.level}"><small>{new Date(l.at * 1000).toLocaleTimeString()}</small> <i>{l.method}</i> {l.msg}</div>
          {/each}
        </div>
      {/if}
    {/if}
  </Modal>
{/if}

<style>
  .note {
    color: var(--mp-sub);
    font-size: 12.5px;
    line-height: 1.6;
    margin: 6px 0;
  }
  .pd-head {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .pd-head small {
    color: var(--mp-sub);
  }
  .grow {
    flex: 1;
  }
  .sw,
  .row {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .pd-err {
    color: #ff8a8a;
    font-size: 13px;
    white-space: pre-line;
    margin: 6px 0;
  }
  .pd-missing {
    display: flex;
    gap: 12px;
    align-items: center;
    color: #ff8a8a;
    font-size: 13px;
    line-height: 1.6;
    margin: 8px 0;
    padding: 10px 12px;
    border-radius: 10px;
    background: rgba(255, 110, 110, 0.1);
  }
  .pd-missing span {
    flex: 1;
  }
  .pd-missing .mp-btn {
    flex-shrink: 0;
  }
  .pd-warn {
    color: #ffb45a;
    font-size: 13px;
    margin: 6px 0;
  }
  .pd-upd {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
    font-size: 13px;
    margin: 8px 0;
  }
  .pd-tabs {
    margin: 12px 0 4px;
  }
  h6 {
    margin: 14px 0 6px;
  }
  .pd-src {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
    font-size: 13px;
  }
  .pd-src small {
    color: var(--mp-sub);
  }
  .st {
    font-size: 11px;
    padding: 1px 6px;
    border-radius: 6px;
  }
  .st.ok {
    background: rgba(80, 200, 120, 0.16);
    color: #6fd896;
  }
  .st.bad {
    background: rgba(255, 110, 110, 0.16);
    color: #ff8a8a;
  }
  .pd-form {
    display: grid;
    gap: 10px;
  }
  .pd-field {
    display: grid;
    gap: 4px;
    font-size: 13px;
  }
  .pd-field small {
    color: var(--mp-sub);
  }
  .pd-field .row .tin {
    flex: 1;
  }
  .pd-perm {
    margin: 0;
    padding-left: 18px;
    font-size: 13px;
    line-height: 1.8;
  }
  .pd-kv div {
    display: flex;
    gap: 10px;
    font-size: 13px;
    padding: 2px 0;
  }
  .pd-kv span {
    color: var(--mp-sub);
    width: 4em;
    flex-shrink: 0;
  }
  .pd-readme pre,
  .pd-stage pre {
    white-space: pre-wrap;
    font-size: 12px;
    max-height: 260px;
    overflow: auto;
    background: rgba(127, 127, 127, 0.08);
    padding: 8px;
    border-radius: 8px;
  }
  .pd-actions {
    flex-wrap: wrap;
  }
  .danger {
    color: #ff8a8a;
  }
  .frow {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
    margin: 8px 0;
  }
  .pd-stage {
    border-left: 3px solid #6fd896;
    padding: 6px 10px;
    margin: 8px 0;
    font-size: 13px;
  }
  .pd-stage.bad {
    border-color: #ff8a8a;
  }
  .pd-stage small {
    color: var(--mp-sub);
  }
  .logs {
    max-height: 360px;
    overflow: auto;
    font-family: ui-monospace, monospace;
    font-size: 12px;
    line-height: 1.6;
  }
  .log small {
    color: var(--mp-sub);
  }
  .log i {
    color: var(--mp-sub);
    font-style: normal;
  }
  .log.error,
  .log.stderr {
    color: #ff8a8a;
  }
  .log.warn {
    color: #ffb45a;
  }
</style>
