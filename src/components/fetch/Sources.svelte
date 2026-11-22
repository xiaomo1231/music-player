<script lang="ts">
  // 我的来源：添加（自动判断类型，网页来源由 AI 生成提取规则）、重建规则、删除
  import { onMount } from 'svelte';
  import Icon from '../../lib/Icon.svelte';
  import { listen, inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { fetchApi, KIND_TEXT, MEDIA_TEXT, type ProviderInfo, type Source, type SourceStep } from '../../lib/fetch';

  let { onchange }: { onchange?: () => void } = $props();

  let list = $state<Source[]>([]);
  let url = $state('');
  let name = $state('');
  let template = $state('');
  let kind = $state('auto');
  let busy = $state(false);
  let steps = $state<SourceStep[]>([]);
  // 已接入的下载源（默认没有）；已经添加过的不再列出
  let providers = $state<ProviderInfo[]>([]);
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
  }

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
      onchange?.();
    } catch (err) {
      toast(String(err));
    } finally {
      busy = false;
    }
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
  <input class="tin" style="flex:1" placeholder="粘贴网站地址或订阅链接" autocomplete="off" bind:value={url} onkeydown={e => e.key === 'Enter' && add()} />
  <button class="mp-btn" onclick={add} disabled={busy}>添加</button>
</div>
{#if addable.length > 0}
  <div class="frow" style="margin-top:10px">
    <span>可添加的下载源</span>
    {#each addable as p (p.id)}
      <button class="mp-btn ghost" title={p.note || p.media_kinds.map(k => MEDIA_TEXT[k] ?? k).join('、')} onclick={() => addProvider(p)} disabled={busy}>{p.name}</button>
    {/each}
  </div>
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
    </select>
  </div>
  <div class="frow" style="margin-top:8px">
    <input class="tin" style="flex:1" placeholder="名字（可不填）" bind:value={name} />
  </div>
  <div class="frow" style="margin-top:8px">
    <input class="tin" style="flex:1" placeholder="搜索网址模板，用 {'{q}'} 代表关键字（可不填）" bind:value={template} />
  </div>
</details>

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

<div class="note" style="margin-top:10px">软件不自带任何来源，全部由你添加。请只添加你有权获取内容的网站。遇到登录或验证码时，请在浏览器里自己完成，软件不会绕过。</div>

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
