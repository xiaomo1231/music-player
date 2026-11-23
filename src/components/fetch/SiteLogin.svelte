<script lang="ts">
  // 在软件里登录网站：打开一个浏览器窗口让用户自己登录，完成后保存这个网站的 Cookie
  import { onMount } from 'svelte';
  import { listen, inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { fetchApi, type LoginEvent } from '../../lib/fetch';

  let { url, ondone }: { url: string; ondone: (saved: boolean) => void } = $props();

  let site = $state('');
  let busy = $state(false);
  let failed = $state('');

  onMount(() => {
    let un: (() => void) | undefined;
    let closed = false;
    if (!inTauri) {
      toast('请在桌面程序里登录');
      ondone(false);
      return;
    }
    // 用户直接关掉登录窗口时，后端也会保存并发这个事件
    listen<LoginEvent>('fetch://login', p => {
      if (closed) return;
      if (p.status === 'saved') {
        closed = true;
        toast(p.message);
        ondone(true);
      } else {
        failed = p.message;
      }
    }).then(f => (un = f));
    fetchApi
      .loginOpen(url)
      .then(s => (site = s))
      .catch(err => {
        toast(String(err));
        closed = true;
        ondone(false);
      });
    return () => {
      closed = true;
      un?.();
    };
  });

  async function finish() {
    busy = true;
    failed = '';
    try {
      const v = await fetchApi.loginFinish();
      if (v.count === 0) failed = '没有读到这个网站的 Cookie，可能还没有登录成功。';
    } catch (err) {
      failed = String(err);
    } finally {
      busy = false;
    }
  }
</script>

<div class="sl note">
  <div>已在新窗口打开 <b>{site || url}</b>。请在那个窗口里自己登录（验证码也在那里完成），登录成功后点“完成登录”，或者直接关掉那个窗口。</div>
  {#if failed}<div class="bad">{failed}</div>{/if}
  <div class="row">
    <button class="mp-btn" onclick={finish} disabled={busy}>{busy ? '读取中…' : '完成登录'}</button>
    <button class="mp-btn ghost" onclick={() => ondone(false)}>先不登录</button>
  </div>
</div>

<style>
  .sl {
    display: grid;
    gap: 8px;
    margin-top: 10px;
  }
  .row {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .bad {
    color: #ff9b9b;
  }
</style>
