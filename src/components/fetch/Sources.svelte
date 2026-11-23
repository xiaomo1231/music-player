<script lang="ts">
  // 我的来源：添加（自动判断类型，网页来源由 AI 生成提取规则）、重建规则、删除
  import { onMount } from 'svelte';
  import Icon from '../../lib/Icon.svelte';
  import SiteLogin from './SiteLogin.svelte';
  import { listen, inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { fetchApi, needsLogin, KIND_TEXT, MEDIA_TEXT, metaApi, type MetaInfo, type ProviderInfo, type Source, type SourceStep } from '../../lib/fetch';
  import { em } from '../../lib/em.svelte';

  let { onchange }: { onchange?: () => void } = $props();

  let list = $state<Source[]>([]);
  let url = $state('');
  let name = $state('');
  let template = $state('');
  let kind = $state('auto');
  let busy = $state(false);
  let steps = $state<SourceStep[]>([]);
  // 添加时遇到要登录的网站：记下地址，提示在软件里登录
  let loginUrl = $state('');
  let loggingIn = $state(false);
  // 已接入的下载源（默认没有）；已经添加过的不再列出
  let providers = $state<ProviderInfo[]>([]);
  // 已接入的资料源（作品资料，默认没有）：不用添加，自动用在搜索页
  let metas = $state<MetaInfo[]>([]);
  const kindsText = (kinds: string[]) => (kinds.length ? kinds.map(k => MEDIA_TEXT[k] ?? k).join('、') : '全部类型');
  let addable = $derived(providers.filter(p => !list.some(s => s.kind === 'provider' && s.url === `provider://${p.id}`)));

  const ICON: Record<string, string> = { rss: 'list', ytdlp: 'video', dir: 'folder', web: 'search', provider: 'download' };
  const BADGE: Record<string, string> = { rules: '规则已生成', ai_direct: 'AI 直接读取' };

  async function load() {
    if (!inTauri) return;
    try {
      list = await fetchApi.sources();
      providers = await fetchApi.providers();
    } catch (err) {
      toast(String(err));
    }
    try {
      metas = await metaApi.providers();
    } catch {
      /* 旧版本没有资料源命令时忽略 */
    }
  }

  // 下载源规则（.em）导入、停用、删除后，可添加的下载源和来源列表跟着变
  let emSeen = em.version;
  $effect(() => {
    const v = em.version;
    if (v === emSeen) return;
    emSeen = v;
    void load();
  });

  onMount(() => {
    load();
    let un: (() => void) | undefined;
    listen<SourceStep>('fetch://source', p => {
      const i = steps.findIndex(s => s.step === p.step);
      if (i >= 0) steps[i] = p;
      else steps = [...steps, p];
    }).then(f => (un = f));
    return () => un?.();
  });

  const stepClass = (s: string) => (s === 'run' ? 'st run' : s === 'ok' ? 'st ok' : s === 'error' ? 'st' : 'st');

  async function add() {
    const u = url.trim();
    if (!u) return toast('先粘贴一个网站地址、订阅链接或文件夹路径');
    if (busy) return;
    busy = true;
    steps = [];
    try {
      const s = await fetchApi.addSource(kind, u, name.trim() || undefined, template.trim() || undefined);
      list = [...list, s];
      url = '';
      name = '';
      template = '';
      toast(`已添加来源：${s.name}`);
      loginUrl = '';
      onchange?.();
    } catch (err) {
      const msg = String(err);
      toast(msg);
      loginUrl = needsLogin(msg) && /^https?:\/\//i.test(u) ? u : '';
    } finally {
      busy = false;
    }
  }

  // 自己写的下载源程序（exe / bat / cmd），协议见 docs/下载源接口.md 第 9 节
  async function pickProgram() {
    if (!inTauri) return toast('请在桌面程序里选择下载源程序');
    const { open } = await import('@tauri-apps/plugin-dialog');
    const p = await open({ multiple: false, filters: [{ name: '下载源程序', extensions: ['exe', 'bat', 'cmd'] }] });
    if (typeof p !== 'string' || !p) return;
    url = p;
    kind = 'exe';
    await add();
    kind = 'auto';
  }

  async function addProvider(p: ProviderInfo) {
    if (busy) return;
    busy = true;
    try {
      const s = await fetchApi.addSource('provider', `provider://${p.id}`, p.name);
      list = [...list, s];
      toast(`已添加下载源：${s.name}`);
      onchange?.();
    } catch (err) {
      toast(String(err));
    } finally {
      busy = false;
    }
  }

  async function rebuild(s: Source) {
    if (busy) return;
    busy = true;
    steps = [];
    try {
      const n = await fetchApi.rebuildRule(s.id);
      list = list.map(x => (x.id === n.id ? n : x));
      toast('已重新生成提取规则');
    } catch (err) {
      toast(String(err));
    } finally {
      busy = false;
    }
  }

  async function remove(s: Source) {
    try {
      await fetchApi.removeSource(s.id);
      list = list.filter(x => x.id !== s.id);
      toast('已删除来源（已下载的文件不受影响）');
      onchange?.();
    } catch (err) {
      toast(String(err));
    }
  }
</script>

<div class="card-head">
  <h5>我的来源</h5>
  <span class="badge2">{list.length} 个</span>
</div>

{#if list.length === 0}
  <div class="empty" style="padding:18px 6px">还没有来源。添加你有权获取内容的网站、订阅链接或自己的网盘目录。</div>
{/if}
<div id="srcs">
  {#each list as s (s.id)}
    <div class="src">
      <Icon name={ICON[s.kind] ?? 'list'} />
      <b>{s.name}</b>
      <span class="row-end">
        {#if s.kind === 'web'}
          <span class="badge2 {s.mode === 'ai_direct' ? 'ai' : 'ok'}">{BADGE[s.mode] ?? s.mode}</span>
          <button class="mp-btn ghost" title="网站改版后可重新生成规则" onclick={() => rebuild(s)} disabled={busy}>重新生成规则</button>
        {:else}
          <span class="badge2 ok">正常</span>
        {/if}
        <button class="mp-btn ghost" title="删除来源" aria-label="删除来源" onclick={() => remove(s)}><Icon name="trash" /></button>
      </span>
      <small>{KIND_TEXT[s.kind] ?? s.kind} · {s.url}</small>
    </div>
  {/each}
</div>

<div class="frow" style="margin-top:10px">
  <input class="tin" style="flex:1" placeholder="粘贴网站地址、订阅链接，或下载源程序（exe）的路径" autocomplete="off" bind:value={url} onkeydown={e => e.key === 'Enter' && add()} />
  <button class="mp-btn" onclick={add} disabled={busy}>添加</button>
  <button class="mp-btn ghost" title="添加自己写的下载源程序：搜索和下载时由软件调用它" onclick={pickProgram} disabled={busy}>选择程序…</button>
</div>
{#if addable.length > 0}
  <div class="frow" style="margin-top:10px">
    <span>可添加的下载源</span>
    {#each addable as p (p.id)}
      <button class="mp-btn ghost" title={(p.note ? p.note + '\n' : '') + '支持：' + kindsText(p.media_kinds)} onclick={() => addProvider(p)} disabled={busy}>{p.name}</button>
    {/each}
  </div>
{/if}
{#if metas.length > 0}
  <div class="frow metas" style="margin-top:10px">
    <span>资料源</span>
    {#each metas as m (m.id)}
      <span class="mp-chip" title={m.note}>{m.name} · {kindsText(m.media_kinds)}</span>
    {/each}
  </div>
  <div class="note" style="margin-top:4px">搜索时会自动用资料源查作品的封面、简介和分集，不用单独添加。</div>
{/if}
<details class="adv">
  <summary>高级选项</summary>
  <div class="frow" style="margin-top:8px">
    <span>类型</span>
    <select class="sel" bind:value={kind}>
      <option value="auto">自动判断</option>
      <option value="rss">订阅链接（RSS / Atom）</option>
      <option value="ytdlp">视频网站（yt-dlp）</option>
      <option value="dir">网盘或 NAS 目录（本地路径、\\服务器\共享、WebDAV 地址）</option>
      <option value="web">其他网页（AI 解析）</option>
      <option value="exe">外部程序下载源（exe / bat / cmd）</option>
    </select>
  </div>
  <div class="frow" style="margin-top:8px">
    <input class="tin" style="flex:1" placeholder="名字（可不填）" bind:value={name} />
  </div>
  <div class="frow" style="margin-top:8px">
    <input class="tin" style="flex:1" placeholder="搜索网址模板，用 {'{q}'} 代表关键字（可不填）" bind:value={template} />
  </div>
</details>

{#if loginUrl && !loggingIn}
  <div class="note" style="margin-top:10px">
    这个网站可能需要登录。可以在软件里打开它，自己登录一次，之后识别链接、搜索和下载都会带上登录状态。
    <div style="margin-top:6px"><button class="mp-btn" onclick={() => (loggingIn = true)}>在软件里登录</button></div>
  </div>
{/if}
{#if loggingIn}
  <SiteLogin url={loginUrl} ondone={saved => { loggingIn = false; if (saved) { url = loginUrl; loginUrl = ''; add(); } }} />
{/if}

{#if steps.length > 0}
  <div class="steps" style="margin-top:10px">
    {#each steps as s (s.step)}
      <span class={stepClass(s.status)} title={s.message}>{s.step}</span>
    {/each}
  </div>
  {#if steps.some(s => s.status === 'warn' || s.status === 'error')}
    <div class="note" style="margin-top:6px">{steps.filter(s => s.status === 'warn' || s.status === 'error').at(-1)?.message}</div>
  {/if}
{/if}

<div class="note" style="margin-top:10px">软件不自带任何来源，全部由你添加。请只添加你有权获取内容的网站。遇到需要登录的网站，可以在软件里打开它自己登录一次（验证码也由你自己完成），软件不会绕过登录或验证码。</div>

<style>
  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 8px;
  }
  .card-head h5 {
    margin: 0;
  }
  .row-end {
    display: flex;
    gap: 6px;
    align-items: center;
    justify-self: end;
    flex-wrap: wrap;
  }
  .metas {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-wrap: wrap;
  }
  .adv {
    margin-top: 8px;
    font-size: 12.5px;
    color: var(--mp-sub);
  }
  .adv summary {
    cursor: pointer;
    padding: 4px 0;
  }
  .adv .frow {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
</style>
