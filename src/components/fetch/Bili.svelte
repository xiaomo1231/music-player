<script lang="ts">
  // B 站：扫码登录、粘贴链接解析出分 P / 剧集 / 收藏夹里的视频，勾选后加入下载列表（完成后自动整理入库）
  import { onMount, onDestroy } from 'svelte';
  import Modal from '../Modal.svelte';
  import { inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { fetchApi, biliApi, type BiliAccount, type BiliParsed, type BiliQr, type YtdlpStatus } from '../../lib/fetch';

  let { onqueued }: { onqueued?: () => void } = $props();

  let account = $state<BiliAccount | null>(null);
  let ytdlp = $state<YtdlpStatus | null>(null);
  let installing = $state(false);

  let qr = $state<BiliQr | null>(null);
  let qrOpen = $state(false);
  let qrState = $state('waiting');
  let timer: ReturnType<typeof setInterval> | undefined;

  let link = $state('');
  let parsing = $state(false);
  let parsed = $state<BiliParsed | null>(null);
  let sel = $state<boolean[]>([]);
  let mk = $state('video');

  const QR_TEXT: Record<string, string> = {
    waiting: '用 B 站手机客户端扫一扫',
    scanned: '已扫码，请在手机上点“确认登录”',
    expired: '二维码已过期',
  };
  const KIND_LABEL: Record<string, string> = { bangumi: '番剧 / 影视', video: '视频', list: '列表' };

  const selCount = $derived(sel.filter(Boolean).length);
  const allOn = $derived(parsed != null && parsed.entries.length > 0 && selCount === parsed.entries.length);

  async function loadAccount() {
    try {
      account = await biliApi.account();
    } catch {
      account = { logged_in: false, expired: false, name: '', vip: false, vip_label: '' };
    }
  }

  async function loadYtdlp() {
    try {
      ytdlp = await biliApi.ytdlpStatus();
    } catch {
      ytdlp = null;
    }
  }

  onMount(() => {
    if (!inTauri) return;
    void loadAccount();
    void loadYtdlp();
  });

  onDestroy(stopPoll);

  function stopPoll() {
    if (timer) clearInterval(timer);
    timer = undefined;
  }

  async function installYtdlp() {
    installing = true;
    try {
      ytdlp = await biliApi.ytdlpInstall();
      toast(`yt-dlp 已就绪${ytdlp.version ? '（' + ytdlp.version + '）' : ''}`);
    } catch (err) {
      toast(String(err));
    } finally {
      installing = false;
    }
  }

  async function startLogin() {
    stopPoll();
    qrOpen = true;
    qrState = 'waiting';
    qr = null;
    try {
      qr = await biliApi.qrStart();
    } catch (err) {
      toast(String(err));
      qrOpen = false;
      return;
    }
    timer = setInterval(poll, 2000);
  }

  async function poll() {
    if (!qr) return;
    try {
      const s = await biliApi.qrPoll(qr.key);
      qrState = s;
      if (s === 'ok') {
        stopPoll();
        qrOpen = false;
        await loadAccount();
        toast(account?.logged_in ? `已登录：${account.name}` : '登录成功');
      } else if (s === 'expired') {
        stopPoll();
      }
    } catch (err) {
      stopPoll();
      toast(String(err));
    }
  }

  function closeQr() {
    stopPoll();
    qrOpen = false;
  }

  async function logout() {
    await biliApi.logout();
    account = { logged_in: false, expired: false, name: '', vip: false, vip_label: '' };
    toast('已退出 B 站账号，登录凭据已从本机删除');
  }

  async function parse() {
    const u = link.trim();
    if (!u) return toast('先粘贴一个 B 站链接');
    if (!inTauri) return;
    parsing = true;
    parsed = null;
    try {
      const p = await biliApi.parse(u);
      parsed = p;
      sel = p.entries.map(() => true);
      mk = p.media_kind;
      if (!p.entries.length) toast('这个链接里没有找到可以下载的视频');
    } catch (err) {
      toast(String(err));
    } finally {
      parsing = false;
    }
  }

  function toggleAll() {
    const v = !allOn;
    sel = sel.map(() => v);
  }

  async function download() {
    if (!parsed) return;
    const items = parsed.entries
      .filter((_, i) => sel[i])
      .map(e => ({ title: e.title, url: e.url, kind: 'ytdlp', mediaKind: mk, sourceId: null, size: null }));
    if (!items.length) return toast('先勾选要下载的条目');
    try {
      await fetchApi.download(items);
      toast(`已加入下载列表：${items.length} 个`);
      onqueued?.();
    } catch (err) {
      toast(String(err));
    }
  }
</script>

<div class="head">
  <h5>B 站</h5>
  <span class="spacer"></span>
  {#if account?.logged_in}
    <span class="who">{account.name}</span>
    {#if account.vip}<span class="badge2 ok">{account.vip_label || '大会员'}</span>{/if}
    <button class="mp-btn ghost" onclick={logout}>退出</button>
  {:else}
    {#if account?.expired}<span class="badge2">登录已失效</span>{/if}
    <button class="mp-btn ghost" onclick={startLogin} disabled={!inTauri}>扫码登录</button>
  {/if}
</div>

{#if ytdlp && !ytdlp.installed}
  <div class="note warn">
    没有找到 yt-dlp（安装包自带的可能被删除了）。点“安装”会从 yt-dlp 官方发布页下载（约 23 MB）并校验后放到软件的数据目录。
    <button class="mp-btn" onclick={installYtdlp} disabled={installing}>{installing ? '下载中…' : '安装 yt-dlp'}</button>
  </div>
{/if}

<div class="frow" style="margin-top:8px">
  <input
    class="tin"
    style="flex:1;min-width:140px"
    placeholder="粘贴视频、番剧、合集、收藏夹或 UP 主空间的链接"
    autocomplete="off"
    bind:value={link}
    onkeydown={e => e.key === 'Enter' && parse()}
  />
  <button class="mp-btn" onclick={parse} disabled={parsing}>{parsing ? '解析中…' : '解析'}</button>
</div>

{#if parsed && parsed.entries.length}
  <div class="res">
    <div class="res-head">
      <b title={parsed.title}>{parsed.title || '解析结果'}</b>
      <span class="badge2">{KIND_LABEL[parsed.kind] ?? parsed.kind} · {parsed.entries.length} 个</span>
    </div>
    <div class="frow">
      <button class="mp-btn ghost" onclick={toggleAll}>{allOn ? '全不选' : '全选'}</button>
      <select class="sel" bind:value={mk} aria-label="存到">
        <option value="anime">存到番剧库</option>
        <option value="video">存到视频库</option>
        <option value="music">只要音频，存到音乐库</option>
      </select>
      <span class="spacer"></span>
      <button class="mp-btn" onclick={download} disabled={selCount === 0}>下载选中（{selCount}）</button>
    </div>
    <div class="list">
      {#each parsed.entries as e, i (e.url + i)}
        <label class="it">
          <input type="checkbox" class="mp-sw" bind:checked={sel[i]} />
          <span>{e.title}</span>
        </label>
      {/each}
    </div>
  </div>
{/if}

<div class="note" style="margin-top:8px">
  {#if account?.logged_in}
    清晰度、弹幕和字幕在右上角“设置”里调。下载完成后自动整理进媒体库。
  {:else}
    不登录也能下载，但最高 480P、没有 CC 字幕。登录凭据只保存在本机的 Windows 凭据管理器里。
  {/if}
  {#if ytdlp?.installed}
    <span class="ver">yt-dlp {ytdlp.version}
      <button class="link" onclick={installYtdlp} disabled={installing}>{installing ? '更新中…' : '更新'}</button></span>
  {/if}
</div>

{#if qrOpen}
  <Modal title="扫码登录 B 站" onclose={closeQr}>
    <div class="qr">
      {#if qr}
        <!-- 二维码 SVG 由后端用 qrcode 库生成 -->
        <div class="qr-img" class:dim={qrState === 'expired'}>{@html qr.svg}</div>
      {:else}
        <div class="qr-img empty">正在获取二维码…</div>
      {/if}
      <p>{QR_TEXT[qrState] ?? ''}</p>
      {#if qrState === 'expired'}
        <button class="mp-btn" onclick={startLogin}>刷新二维码</button>
      {/if}
      <div class="note">只用于下载你的账号本来就能看的内容；软件不会绕过会员或付费限制。</div>
    </div>
  </Modal>
{/if}

<style>
  .head {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  .head h5 {
    margin: 0;
  }
  .who {
    font-size: 13px;
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .spacer {
    flex: 1;
  }
  .warn {
    margin-top: 8px;
    display: flex;
    gap: 10px;
    align-items: center;
    flex-wrap: wrap;
  }
  .res {
    margin-top: 12px;
    display: grid;
    gap: 8px;
  }
  .res-head {
    display: flex;
    gap: 8px;
    align-items: center;
    min-width: 0;
    font-size: 13.5px;
  }
  .res-head b {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .list {
    max-height: 300px;
    overflow: auto;
    display: grid;
    gap: 4px;
  }
  .it {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 7px 10px;
    border-radius: 9px;
    background: rgba(255, 255, 255, 0.05);
    font-size: 13px;
    cursor: pointer;
  }
  .it span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ver {
    display: block;
    margin-top: 4px;
    opacity: 0.8;
  }
  .link {
    background: none;
    border: 0;
    color: var(--mp-accent);
    cursor: pointer;
    font: inherit;
    padding: 0 4px;
  }
  .qr {
    display: grid;
    justify-items: center;
    gap: 10px;
    text-align: center;
  }
  .qr-img {
    width: 220px;
    height: 220px;
    background: #fff;
    border-radius: 12px;
    padding: 8px;
    display: grid;
    place-items: center;
    color: #333;
    font-size: 13px;
  }
  .qr-img :global(svg) {
    width: 100%;
    height: 100%;
  }
  .qr-img.dim {
    opacity: 0.25;
  }
  .qr p {
    margin: 0;
    font-size: 14px;
  }
</style>
