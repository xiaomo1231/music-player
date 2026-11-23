<script lang="ts">
  // 导入 .em 规则前的确认框：列出规则会做什么、要访问哪些网站；有错误时显示行号
  import Modal from '../Modal.svelte';
  import { toast } from '../../lib/ui.svelte';
  import { em, emInspect, emInstall, issueText, type EmPreview } from '../../lib/em.svelte';

  const path = $derived(em.pending[0] ?? '');
  let p = $state.raw<EmPreview | null>(null);
  let err = $state('');
  let busy = $state(false);

  $effect(() => {
    const cur = path;
    p = null;
    err = '';
    if (!cur) return;
    emInspect(cur)
      .then(r => {
        if (path === cur) p = r;
      })
      .catch(e => {
        if (path === cur) err = String(e);
      });
  });

  function next() {
    em.pending = em.pending.slice(1);
  }

  async function install() {
    if (!p?.ok) return;
    busy = true;
    try {
      const r = await emInstall(path);
      em.version++;
      toast(r.problem ? `已导入「${r.name}」，但：${r.problem}` : `已${p.installed_version ? '更新' : '导入'}规则「${r.name}」`);
      next();
    } catch (e) {
      toast(String(e));
    } finally {
      busy = false;
    }
  }

  const fileName = $derived(path.split(/[\\/]/).pop() ?? '');
  const LOGIN = { none: '', optional: '登录过的话会带上你在“网站登录”里保存的 Cookie', required: '需要你先在“网站登录”里登录这个网站' };
</script>

{#if path}
  <Modal title={p?.installed_version ? '更新规则' : '导入规则'} onclose={next}>
    {#if err}
      <p class="em-err">{err}</p>
      <div class="rowbtns"><button class="mp-btn" onclick={next}>关闭</button></div>
    {:else if !p}
      <p class="note">正在读取 {fileName}…</p>
    {:else if !p.ok}
      <p>「{p.name || fileName}」有问题，不能导入：</p>
      <ul class="em-issues">
        {#each p.issues as i, k (k)}<li>{issueText(i)}</li>{/each}
      </ul>
      <div class="rowbtns"><button class="mp-btn" onclick={next}>关闭</button></div>
    {:else}
      <div class="em-head">
        <span class="badge2">{p.kind_label}</span>
        <b>{p.name}</b>
        {#if p.version}<small>v{p.version}</small>{/if}
      </div>
      {#if p.installed_version}
        <p class="note">已装版本 v{p.installed_version || '?'}，导入后会替换成这个文件。</p>
      {/if}
      <div class="em-kv">
        {#if p.author}<div><span>作者</span>{p.author}</div>{/if}
        {#if p.note}<div><span>说明</span>{p.note}</div>{/if}
        <div><span>id</span>{p.id}</div>
      </div>
      {#if p.summary.length}
        <h6>这个规则会</h6>
        <ul class="em-sum">
          {#each p.summary as s, k (k)}<li>{s}</li>{/each}
        </ul>
      {/if}
      {#if p.hosts.length}
        <h6>会访问这些网站</h6>
        <ul class="em-hosts">
          {#each p.hosts as h (h)}
            <li>
              {h}
              {#if p.lan_hosts.includes(h)}<span class="tag warn">本机 / 局域网</span>{/if}
              {#if p.new_hosts.includes(h) && p.installed_version}<span class="tag warn">新增</span>{/if}
            </li>
          {/each}
        </ul>
        {#if LOGIN[p.login]}<p class="note">{LOGIN[p.login]}。</p>{/if}
        <p class="note">只导入来源有权获取内容的规则。规则不能自己登录、过验证码或解密受保护的内容。</p>
      {:else}
        <p class="note">这个规则不联网，只处理你电脑上的文件。</p>
      {/if}
      <div class="rowbtns">
        <button class="mp-btn ghost" onclick={next}>取消</button>
        <button class="mp-btn" disabled={busy} onclick={install}>{p.installed_version ? '更新' : '导入'}</button>
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
  .em-head {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 15px;
    margin-bottom: 6px;
  }
  .em-head small {
    color: var(--mp-sub);
  }
  .em-kv div {
    display: flex;
    gap: 10px;
    font-size: 13px;
    padding: 2px 0;
  }
  .em-kv span {
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
  .em-issues,
  .em-err {
    color: #ff8a8a;
    white-space: pre-line;
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
</style>
