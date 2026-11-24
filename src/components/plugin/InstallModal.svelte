<script lang="ts">
  // 安装插件前的确认框：插件会做什么、访问哪些网站、要不要交出登录、用哪些工具；有问题时列出原因
  import Modal from '../Modal.svelte';
  import { toast } from '../../lib/ui.svelte';
  import { MEDIA_TEXT } from '../../lib/fetch';
  import { importEm } from '../../lib/em.svelte';
  import { plugins, pluginApi, RUNTIME_TEXT, TOOL_TEXT, type PluginPreview } from '../../lib/plugin.svelte';

  const path = $derived(plugins.pending[0] ?? '');
  const repo = $derived(plugins.pendingRepo);
  const key = $derived(path || (repo ? `${repo.repo}#${repo.id}` : ''));
  let p = $state.raw<PluginPreview | null>(null);
  let err = $state('');
  let busy = $state(false);
  let allowLogin = $state(false);
  let agreed = $state(false);

  $effect(() => {
    const cur = key;
    p = null;
    err = '';
    allowLogin = false;
    agreed = false;
    if (!cur) return;
    const load = path
      ? pluginApi.inspect(path).then(r => {
          if (r.kind === 'em') {
            importEm([path]);
            next();
            return null;
          }
          if (r.kind === 'exe') throw new Error('这是下载源程序，请在“自动获取 → 我的来源”里点“选择程序…”添加');
          return r.preview;
        })
      : pluginApi.repoInspect(repo!.repo, repo!.id);
    load
      .then(r => {
        if (key === cur && r) {
          p = r;
          allowLogin = r.login === 'required';
        }
      })
      .catch(e => {
        if (key === cur) err = String(e instanceof Error ? e.message : e);
      });
  });

  function next() {
    if (path) plugins.pending = plugins.pending.slice(1);
    else plugins.pendingRepo = null;
  }

  const full = $derived(p?.trust === 'full');

  async function install() {
    if (!p?.ok || (full && !agreed)) return;
    busy = true;
    try {
      const v = repo && !path ? await pluginApi.repoInstall(repo.repo, repo.id, allowLogin) : await pluginApi.install(path, allowLogin);
      plugins.version++;
      const where = v.sources.length || v.rules.length ? '，下载源已加到“自动获取 → 我的来源”' : '';
      if (v.problem) toast(`已安装「${v.name}」，但：${v.problem}`);
      else toast(`已${p.installed_version ? '更新' : '安装'}插件「${v.name}」${where}`);
      next();
      // 有必填设置没填：直接打开设置
      if (v.needs_settings.length) plugins.detail = v.id;
    } catch (e) {
      toast(String(e));
    } finally {
      busy = false;
    }
  }

  const fileName = $derived(path.split(/[\\/]/).pop() ?? repo?.id ?? '');
  const kinds = $derived((p?.media_kinds ?? []).map(k => MEDIA_TEXT[k] ?? k).join('、'));
</script>

{#if key}
  <Modal title={p?.installed_version ? '更新插件' : '安装插件'} onclose={next}>
    {#if err}
      <p class="pl-err">{err}</p>
      <div class="rowbtns"><button class="mp-btn" onclick={next}>关闭</button></div>
    {:else if !p}
      <p class="note">正在读取 {fileName}…</p>
    {:else if !p.ok}
      <p>「{p.name || fileName}」不能安装：</p>
      <ul class="pl-err">
        {#each p.issues as i, k (k)}<li>{i}</li>{/each}
      </ul>
      <div class="rowbtns"><button class="mp-btn" onclick={next}>关闭</button></div>
    {:else}
      <div class="pl-head">
        <span class="badge2">{RUNTIME_TEXT[p.runtime] ?? p.runtime}</span>
        <b>{p.name}</b>
        {#if p.version}<small>v{p.version}</small>{/if}
      </div>
      {#if p.installed_version}
        <p class="note">
          已装 v{p.installed_version}{#if p.downgrade}<span class="tag warn">这是更旧的版本</span>{/if}{#if p.dev_installed}<span class="tag warn">会替换开发者模式加载的文件夹</span>{/if}
        </p>
      {/if}
      <div class="pl-kv">
        {#if p.author}<div><span>作者</span>{p.author}</div>{/if}
        {#if p.description}<div><span>说明</span>{p.description}</div>{/if}
        <div><span>id</span>{p.id}</div>
      </div>

      <h6>会提供</h6>
      <ul>
        {#if p.sources.length}<li>下载源：{p.sources.join('、')}{kinds ? `（${kinds}）` : ''}，装好后出现在“自动获取 → 我的来源”</li>{/if}
        {#each p.converters ?? [] as c (c)}<li>格式转换：{c}，装好后出现在“格式转换”页</li>{/each}
        {#each p.rules as r (r)}<li>规则文件 {r}</li>{/each}
        {#if p.settings.length}<li>{p.settings.length} 项设置{p.settings.some(s => s.required) ? '（有必填项，装好后填写）' : ''}</li>{/if}
      </ul>

      {#if p.new_permissions.length}
        <h6>新版本多要这些权限</h6>
        <ul class="warnlist">
          {#each p.new_permissions as n (n)}<li>{n}</li>{/each}
        </ul>
      {/if}

      <h6>会访问这些网站</h6>
      {#if p.hosts.length}
        <ul>
          {#each p.hosts as h (h)}
            <li>{h}{#if p.lan_hosts.includes(h)}<span class="tag warn">本机 / 局域网</span>{/if}</li>
          {/each}
        </ul>
      {:else}
        <p class="note">不联网。</p>
      {/if}
      {#if p.tools.length}
        <p class="note">会用到软件里的工具：{p.tools.map(t => TOOL_TEXT[t] ?? t).join('、')}</p>
      {/if}

      {#if p.login !== 'none'}
        <label class="chk">
          <input type="checkbox" bind:checked={allowLogin} />
          把我在“网站登录”里保存的这些网站的登录交给插件{p.login === 'required' ? '（这个插件需要登录才能用）' : '（可选）'}
        </label>
        <p class="note">插件只能用你自己登录后保存的状态，不能替你登录、过验证码。随时可以在插件详情里收回。</p>
      {/if}

      {#if full}
        <div class="full">
          <b>这是一个程序插件</b>：它在你的电脑上运行，能读写文件、访问网络，软件没法限制它。只安装你信任的作者提供的插件。
          <label class="chk"><input type="checkbox" bind:checked={agreed} /> 我信任这个插件的来源</label>
        </div>
      {:else}
        <p class="note">这个插件在软件的沙箱里运行，只能访问上面列出的网站，不能读写你的文件。</p>
      {/if}
      <p class="note">只安装能合法获取内容的插件。插件不能绕过登录、验证码或 DRM 保护。</p>

      <div class="rowbtns">
        <button class="mp-btn ghost" onclick={next}>取消</button>
        <button class="mp-btn" disabled={busy || (full && !agreed)} onclick={install}>{busy ? '正在安装…' : p.installed_version ? '更新' : '安装'}</button>
      </div>
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
  .pl-head {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 15px;
    margin-bottom: 6px;
  }
  .pl-head small {
    color: var(--mp-sub);
  }
  .pl-kv div {
    display: flex;
    gap: 10px;
    font-size: 13px;
    padding: 2px 0;
  }
  .pl-kv span {
    color: var(--mp-sub);
    width: 3em;
    flex-shrink: 0;
  }
  h6 {
    margin: 12px 0 4px;
  }
  ul {
    margin: 0;
    padding-left: 18px;
    font-size: 13px;
    line-height: 1.7;
  }
  .pl-err {
    color: #ff8a8a;
    white-space: pre-line;
  }
  .warnlist {
    color: #ffb45a;
  }
  .tag {
    font-size: 11px;
    padding: 1px 6px;
    border-radius: 6px;
    margin-left: 6px;
  }
  .tag.warn {
    background: rgba(255, 170, 60, 0.18);
    color: #ffb45a;
  }
  .chk {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    font-size: 13px;
    margin: 10px 0 2px;
    cursor: pointer;
  }
  .full {
    margin-top: 12px;
    padding: 10px 12px;
    border-radius: 10px;
    background: rgba(255, 170, 60, 0.1);
    font-size: 13px;
    line-height: 1.6;
  }
</style>
