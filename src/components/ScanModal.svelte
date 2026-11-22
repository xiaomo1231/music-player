<script lang="ts">
  // 扫描向导：选磁盘或文件夹 → 枚举 → 读取信息，实时显示进度
  import Modal from './Modal.svelte';
  import Icon from '../lib/Icon.svelte';
  import { invoke, listen, inTauri, type Drive, type Root, type ScanProgress } from '../lib/api';
  import { ui, toast } from '../lib/ui.svelte';
  import { lib } from '../lib/library.svelte';
  import { fmtSize } from '../lib/util';
  import { onMount } from 'svelte';

  let drives = $state<Drive[]>([]);
  let roots = $state<Root[]>([]);
  let selDrives = $state<Set<string>>(new Set());
  let running = $state(false);
  let pr = $state<ScanProgress | null>(null);
  let watch = $state(true);
  const KIND: Record<string, string> = { music: '音乐', video: '视频', anime: '番剧', manga: '漫画', novel: '小说' };

  onMount(() => {
    if (!inTauri) return;
    invoke<Drive[]>('list_drives').then(d => (drives = d));
    loadRoots();
    invoke<[boolean, ScanProgress]>('scan_status').then(([r, p]) => {
      running = r;
      if (r) pr = p;
    });
    invoke<boolean | null>('get_setting', { key: 'watch' }).then(v => (watch = v ?? true));
    const off = listen<ScanProgress>('scan://progress', p => {
      pr = p;
      running = p.phase === 'enum' || p.phase === 'read';
      if (p.phase === 'done') {
        lib.refresh();
        toast(`扫描完成：新增 ${p.added}，更新 ${p.updated}${p.missing ? `，${p.missing} 个文件已不在原处` : ''}`);
      }
    });
    return () => off.then(f => f());
  });

  async function loadRoots() {
    roots = await invoke<Root[]>('list_roots');
  }
  async function addFolder() {
    const { open } = await import('@tauri-apps/plugin-dialog');
    const dir = await open({ directory: true, multiple: true });
    if (!dir) return;
    for (const d of Array.isArray(dir) ? dir : [dir]) await invoke('add_root', { path: d, watch: true });
    loadRoots();
  }
  async function removeRoot(id: number) {
    await invoke('remove_root', { id });
    loadRoots();
  }
  function toggleDrive(r: string) {
    const s = new Set(selDrives);
    s.has(r) ? s.delete(r) : s.add(r);
    selDrives = s;
  }
  async function start() {
    // 选中的整盘也加入扫描目录（实时监控跟着生效）
    for (const d of selDrives) await invoke('add_root', { path: d, watch: true });
    await loadRoots();
    if (!roots.length) return toast('先添加一个文件夹或选择磁盘');
    await invoke('set_setting', { key: 'watch', value: watch });
    await invoke('start_scan', {});
    running = true;
    pr = null;
  }
  async function setWatch() {
    await invoke('set_setting', { key: 'watch', value: watch });
  }
  const pct = $derived(pr && pr.total ? pr.done / pr.total : 0);
</script>

<Modal title="扫描媒体库" onclose={() => ui.close()} wide>
  {#if !running && (!pr || pr.phase === 'done' || pr.phase === 'canceled')}
    <p class="mp-desc">选择要扫描的文件夹或整个磁盘。只按扩展名快速枚举，再在后台读取标签和封面；系统目录、程序目录、node_modules、回收站会自动跳过。不会修改任何原始文件。</p>
    <h4 style="margin:6px 0">已添加的文件夹</h4>
    {#if !roots.length}<div class="note">还没有添加。点下面的“添加文件夹”，或者选择整个磁盘。</div>{/if}
    {#each roots as r (r.id)}
      <div class="frow" style="margin:4px 0"><Icon name="folder" /><code class="pth">{r.path}</code><button class="mp-chip" onclick={() => removeRoot(r.id)}>移除</button></div>
    {/each}
    <div class="rowbtns" style="margin-top:8px"><button class="mp-btn ghost" onclick={addFolder}>＋ 添加文件夹</button></div>
    <h4 style="margin:16px 0 4px">或者扫描整个磁盘</h4>
    <div class="mp-drives">
      {#each drives as d (d.root)}
        {@const used = d.total ? 1 - d.free / d.total : 0}
        <button class="mp-drive" class:on={selDrives.has(d.root) || roots.some(r => r.path === d.root)} onclick={() => toggleDrive(d.root)}>
          <b>{d.root.replace('\\', '')}</b> {d.label}
          <small>{fmtSize(d.total)} · 可用 {fmtSize(d.free)}</small>
          <div class="mp-dbar"><i style="width:{used * 100}%"></i></div>
        </button>
      {/each}
    </div>
    <label class="mp-opt"><div>实时监控<small>已添加的文件夹里出现新文件时几秒内自动入库，删除的自动标记</small></div><input type="checkbox" class="mp-sw" bind:checked={watch} onchange={setWatch} /></label>
    {#if pr?.phase === 'done'}
      <div class="note">上次扫描：找到 {pr.total} 个文件，新增 {pr.added}，更新 {pr.updated}{pr.missing ? `，${pr.missing} 个已不在原处` : ''}。</div>
    {/if}
    <div class="rowbtns"><button class="mp-btn" onclick={start}>开始扫描</button><button class="mp-btn ghost" onclick={() => ui.close()}>关闭</button></div>
  {:else}
    <div class="mp-scanrun">
      <div class="mp-radar">
        <svg viewBox="0 0 100 100" style="width:100%;height:100%;transform:rotate(-90deg)">
          <circle cx="50" cy="50" r="44" fill="none" stroke="var(--mp-line)" stroke-width="6" />
          <circle cx="50" cy="50" r="44" fill="none" stroke="var(--mp-accent)" stroke-width="6" stroke-linecap="round"
            stroke-dasharray={2 * Math.PI * 44} stroke-dashoffset={2 * Math.PI * 44 * (1 - (pr?.phase === 'enum' ? 0.08 : pct))} style="transition:stroke-dashoffset .3s" />
        </svg>
        <b>{pr?.phase === 'enum' ? '…' : Math.round(pct * 100) + '%'}</b>
      </div>
      <div style="min-width:0">
        <div style="font-size:14px">{pr?.phase === 'enum' ? '正在查找媒体文件' : '正在读取标签、时长和封面'}</div>
        <div class="mp-path">{pr?.path ?? ''}</div>
        <div class="mp-counts">
          {#each ['music', 'video', 'anime', 'manga', 'novel'].slice(0, 4) as k, i}<div><b>{pr?.found[i] ?? 0}</b>{KIND[k]}</div>{/each}
        </div>
        <div style="font-size:12px;color:var(--mp-sub);margin-top:6px">小说 {pr?.found[4] ?? 0} · 已处理 {pr?.done ?? 0} / {pr?.total ?? 0}</div>
      </div>
    </div>
    <div class="mp-feed">
      {#each pr?.recent ?? [] as [k, n] (k + n)}<div>{n}<span>{KIND[k]}</span></div>{/each}
    </div>
    <div class="rowbtns"><button class="mp-btn ghost" onclick={() => invoke('cancel_scan')}>停止扫描</button><button class="mp-btn ghost" onclick={() => ui.close()}>在后台继续</button></div>
  {/if}
</Modal>
