<script lang="ts">
  import { onMount } from 'svelte';
  import { open } from '@tauri-apps/plugin-dialog';
  import { listen, inTauri } from '../lib/api';
  import Modal from '../components/Modal.svelte';
  import OrgRow from '../components/organize/OrgRow.svelte';
  import OrgHistory from '../components/organize/OrgHistory.svelte';
  import { org, MODES, MODE_LABEL, planCount, type OrgBatch, type OrgItem, type OrgProgress } from '../lib/organize';
  import { toast } from '../lib/ui.svelte';

  const RANGE: [string, string][] = [
    ['all', '全部'],
    ['video', '番剧与视频'],
    ['music', '音乐'],
  ];
  const GROUPS: { c: string; label: string }[] = [
    { c: 'hi', label: '把握高 · 默认勾选' },
    { c: 'mid', label: '需要你看一眼' },
    { c: 'low', label: '把握低 · 默认不勾选，认不出的保持原位' },
  ];
  /** 显示用的默认模板（可以在设置里改） */
  const SAMPLE = '番剧/{作品名} ({年份})/Season {季:02}/{作品名} - S{季:02}E{集:02}.ext';

  let types = $state('all');
  let roots = $state<string[]>([]);
  let target = $state('');
  let mode = $state('move');
  let subs = $state(true);
  let cleanup = $state(true);

  let items = $state<OrgItem[]>([]);
  let doneSrcs = $state<string[]>([]);
  let steps = $state<{ name: string; state: 'run' | 'ok' }[]>([]);
  let statusText = $state('尚未开始');
  let statusCls = $state('');
  let busy = $state(false);
  let lastBatch = $state<number | null>(null);
  let batches = $state<OrgBatch[]>([]);
  let confirmAi = $state<{ files: number; tokens: number } | null>(null);
  let confirmExec = $state<{ files: number } | null>(null);

  const planned = $derived(planCount(items, subs));
  const unrecognized = $derived(items.filter(i => !i.dst).length);

  function setStatus(text: string, cls = '') {
    statusText = text;
    statusCls = cls;
  }

  async function addRoot() {
    if (!inTauri) return toast('在桌面程序里才能选择文件夹');
    const r = await open({ directory: true, multiple: false });
    if (typeof r === 'string' && !roots.includes(r)) roots = [...roots, r];
  }

  async function pickTarget() {
    if (!inTauri) return toast('在桌面程序里才能选择文件夹');
    const r = await open({ directory: true, multiple: false });
    if (typeof r === 'string') target = r;
  }

  async function start() {
    if (!inTauri) return toast('在桌面程序里才能识别文件');
    if (!roots.length) return toast('请先添加要整理的文件夹');
    if (!target) return toast('请选择整理到的文件夹');
    busy = true;
    items = [];
    doneSrcs = [];
    lastBatch = null;
    steps = ['扫描文件夹', '解析文件名和时长', '生成整理方案'].map((name): { name: string; state: 'run' | 'ok' } => ({ name, state: 'run' }));
    setStatus('正在识别…', 'ai');
    try {
      items = await org.scan(roots, types, target);
      steps = steps.map((s): { name: string; state: 'run' | 'ok' } => ({ name: s.name, state: 'ok' }));
      setStatus(items.length ? `识别完成 · ${items.length} 个文件` : '没有找到可以整理的文件', items.length ? 'ok' : '');
    } catch (e) {
      steps = [];
      setStatus(String(e), 'no');
    }
    busy = false;
  }

  async function askAi() {
    if (!items.length || busy) return;
    try {
      const est = await org.estimate(items);
      if (!est.files) return toast('没有需要交给 AI 的文件');
      confirmAi = est;
    } catch (e) {
      toast(String(e));
    }
  }

  async function runAi() {
    confirmAi = null;
    busy = true;
    setStatus('AI 识别剩余的文件…', 'ai');
    try {
      items = await org.aiIdentify(items, target);
      setStatus(`AI 识别完成 · 还有 ${items.filter(i => !i.dst).length} 个未识别`, 'ok');
    } catch (e) {
      toast(String(e));
      setStatus('AI 识别没有完成', 'no');
    }
    busy = false;
  }

  function askExecute() {
    if (!planned) return toast('没有勾选要整理的文件');
    confirmExec = { files: planned };
  }

  async function runExecute() {
    confirmExec = null;
    busy = true;
    setStatus('正在整理…', 'ai');
    const payload = subs ? items : items.map(i => ({ ...i, extras: [] }));
    try {
      const rep = await org.execute(payload, mode, cleanup);
      const failed = new Set(rep.failed.map(f => f.path));
      doneSrcs = items.filter(i => i.checked && i.dst && !failed.has(i.src)).map(i => i.src);
      lastBatch = rep.batch_id;
      setStatus(`完成 · 已整理 ${rep.done} 个文件，没有删除任何文件`, 'ok');
      if (rep.failed.length) toast(`${rep.failed.length} 个文件没有整理（多半被占用），已跳过`);
      batches = await org.batches();
    } catch (e) {
      toast(String(e));
      setStatus('整理没有完成', 'no');
    }
    busy = false;
  }

  async function undo(id: number) {
    busy = true;
    try {
      const r = await org.undo(id);
      toast(`已撤销 ${r.reverted} 个文件，回到原来的位置和名字`);
      if (r.failed.length) toast(`${r.failed.length} 个文件没能撤销，请检查后再试`);
      if (lastBatch === id) {
        lastBatch = null;
        doneSrcs = [];
      }
      batches = await org.batches();
      setStatus(r.finished ? '已撤销这一批' : '部分文件已撤销', r.finished ? '' : 'no');
    } catch (e) {
      toast(String(e));
    }
    busy = false;
  }

  function undoLast() {
    if (lastBatch != null) undo(lastBatch);
  }

  onMount(() => {
    let stop: (() => void) | undefined;
    listen<OrgProgress>('org://progress', p => {
      if (busy && p.phase !== 'done') setStatus(`正在处理 ${p.done + 1}/${p.total}`, 'ai');
    }).then(f => (stop = f));
    if (inTauri) org.batches().then(b => (batches = b)).catch(() => {});
    return () => stop?.();
  });
</script>

<section class="lib on">
  <header>
    <div>
      <h2>整理文件</h2>
      <p>认出每个文件是什么，改成统一的名字，放进分好类的文件夹 · 先预览再执行，不删除任何文件，可以整批撤销</p>
    </div>
  </header>

  <div class="get" style="margin-top:14px">
    <div class="card">
      <h5>整理设置</h5>
      <div class="frow">
        <span>范围</span>
        <div class="seg">
          {#each RANGE as [v, l]}
            <button class:on={types === v} onclick={() => (types = v)}>{l}</button>
          {/each}
        </div>
      </div>
      <div class="frow">
        <span>来源</span>
        <div style="display:flex;flex-direction:column;gap:6px;flex:1;min-width:0">
          {#each roots as r (r)}
            <div class="frow" style="gap:6px;margin:0">
              <code class="pth">{r}</code>
              <button class="mp-btn ghost" onclick={() => (roots = roots.filter(x => x !== r))}>移除</button>
            </div>
          {:else}
            <code class="pth">还没有添加文件夹</code>
          {/each}
          <button class="mp-btn ghost" style="align-self:flex-start" onclick={addRoot}>添加文件夹</button>
        </div>
      </div>
      <div class="frow">
        <span>整理到</span>
        <code class="pth">{target || '还没有选择'}</code>
        <button class="mp-btn ghost" onclick={pickTarget}>选择</button>
      </div>
      <div class="frow">
        <span>方式</span>
        <select class="sel" bind:value={mode} style="flex:1;min-width:0">
          {#each MODES as [v, l]}
            <option value={v}>{l}</option>
          {/each}
        </select>
      </div>
      {#if mode === 'library'}
        <div class="note">当前版本：这种方式只记录整理方案，硬盘和媒体库里的文件都不改动。</div>
      {/if}
      <div class="frow">
        <span>命名</span>
        <code class="pth">{SAMPLE}</code>
      </div>
      <div class="frow">
        <label style="display:flex;gap:8px;align-items:center">
          <input type="checkbox" class="mp-sw" bind:checked={subs} />字幕、封面等配套文件一起移动
        </label>
      </div>
      <div class="frow">
        <label style="display:flex;gap:8px;align-items:center">
          <input type="checkbox" class="mp-sw" bind:checked={cleanup} />整理后删除变空的旧文件夹（只删完全为空的）
        </label>
      </div>
      <div style="margin-top:12px">
        <button class="mp-btn" onclick={start} disabled={busy}>开始识别</button>
      </div>
      <div class="note" style="margin-top:12px">只发送文件名给 AI，不发送文件内容。多数文件在本地就能认出来，认不出的才交给 AI。</div>
    </div>

    <div class="card">
      <h5>整理方案 <span class="badge2 {statusCls}">{statusText}</span></h5>
      {#if steps.length}
        <div class="steps">
          {#each steps as s (s.name)}
            <span class="st {s.state}">{s.name}</span>
          {/each}
        </div>
      {/if}

      {#if items.length}
        {#each GROUPS as g (g.c)}
          {@const rows = items.filter(i => i.confidence === g.c || (g.c === 'low' && i.confidence !== 'hi' && i.confidence !== 'mid'))}
          {#if rows.length}
            <div class="og">{g.label} <span class="badge2">{rows.length}</span></div>
            {#each rows as it (it.src)}
              <OrgRow item={it} done={doneSrcs.includes(it.src)} />
            {/each}
          {/if}
        {/each}
        <div class="note" style="margin-top:10px">
          共 {items.length} 个文件，其中 {unrecognized} 个认不出（保持原位）。点新路径可以直接修改。
        </div>
      {:else}
        <div class="empty">
          选好文件夹和整理到的位置，点“开始识别”查看整理方案。<br />整理前会列出每个文件的原位置和新位置，确认后才会执行。
        </div>
      {/if}

      <div class="acts" style="margin-top:12px">
        {#if items.length}
          <button class="mp-btn ghost" onclick={askAi} disabled={busy || !unrecognized}>交给 AI 识别剩余的</button>
          <button class="mp-btn" onclick={askExecute} disabled={busy || !planned}>执行整理（{planned} 项）</button>
        {/if}
        {#if lastBatch != null}
          <button class="mp-btn ghost" onclick={undoLast} disabled={busy}>撤销这一批</button>
        {/if}
      </div>

      <div style="margin-top:16px">
        <OrgHistory {batches} {busy} onundo={undo} />
      </div>
    </div>
  </div>
</section>

{#if confirmAi}
  <Modal title="交给 AI 识别" onclose={() => (confirmAi = null)}>
    <p>将把 {confirmAi.files} 个认不出的视频文件名和所在文件夹名发给 AI（不发送文件内容），预计消耗约 {confirmAi.tokens} 个 token。</p>
    <p class="note">AI 只给出整理建议，把握最高是“需要看一眼”，你确认执行前不会改动任何文件。</p>
    <div class="acts" style="justify-content:flex-end;margin-top:14px">
      <button class="mp-btn ghost" onclick={() => (confirmAi = null)}>取消</button>
      <button class="mp-btn" onclick={runAi}>发送并识别</button>
    </div>
  </Modal>
{/if}

{#if confirmExec}
  <Modal title="确认整理" onclose={() => (confirmExec = null)}>
    <p>将以“{MODE_LABEL[mode]}”的方式处理 <b>{confirmExec.files}</b> 个文件（含勾选的配套文件）。</p>
    <p class="note">
      不会删除任何文件。这一批可以整批撤销；撤销复制或硬链接时，我们建的副本会移到回收站。{mode === 'library'
        ? '这种方式只记录方案，硬盘和媒体库都不改动。'
        : ''}
    </p>
    <div class="acts" style="justify-content:flex-end;margin-top:14px">
      <button class="mp-btn ghost" onclick={() => (confirmExec = null)}>取消</button>
      <button class="mp-btn" onclick={runExecute}>确认执行</button>
    </div>
  </Modal>
{/if}
