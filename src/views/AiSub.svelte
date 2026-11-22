<script lang="ts">
  // AI 字幕：左边任务设置，右边处理进度、时间轴和字幕列表（可直接改译文）
  import { onMount } from 'svelte';
  import { ui, toast } from '../lib/ui.svelte';
  import { inTauri } from '../lib/api';
  import {
    aiApi, subApi, listenSubProgress, isLocalBase, tcLabel, SUB_STEPS, STEP_INDEX, SUBS_LABEL,
    type AiState, type JobView, type SubEstimate, type SubLine, type SubProgress, type SubVideo,
  } from '../lib/aisub';

  type StepState = '' | 'run' | 'ok';
  type JobInfo = { step: string; progress: number; message: string };

  let videos = $state<SubVideo[]>([]);
  let ai = $state<AiState | null>(null);
  let fileId = $state<number | null>(null);
  let lang = $state('auto');
  let target = $state('both');
  let output = $state('ass');
  let batch = $state(false);
  let glossary = $state('');
  let est = $state<SubEstimate | null>(null);
  let overOk = $state(false);

  let running = $state(false);
  let jobId = $state<number | null>(null);
  let batchIds = $state<number[]>([]);
  let batchViews = $state<JobView[]>([]);
  let jobInfo = $state<Record<number, JobInfo>>({});
  let steps = $state<StepState[]>(['', '', '', '', '', '']);
  let prog = $state(0);
  let msg = $state('');
  let lines = $state<SubLine[]>([]);
  let outputs = $state<string[]>([]);
  let done = $state(false);
  let canvas = $state<HTMLCanvasElement>();

  const selected = $derived(videos.find(v => v.id === fileId) ?? null);
  const hasAi = $derived(!!ai && (ai.has_key || isLocalBase(ai.config.base_url)));
  const estText = $derived.by(() => {
    if (!selected) return '先选择一个视频。';
    if (!est) return '正在估算…';
    const used = est.used_tokens.toLocaleString();
    const limit = est.monthly_limit ? ` / ${est.monthly_limit.toLocaleString()}` : '';
    const over = est.over_limit ? ' 本月用量会超过上限，开始前会再确认一次。' : '';
    return `预计约 ${est.minutes} 分钟音频，翻译用量约 ${est.est_tokens.toLocaleString()} tokens（按时长估算的上限）。本月已用 ${used}${limit}。${over}`;
  });

  function label(v: SubVideo): string {
    const head = v.series_title ? `${v.series_title} · ` : '';
    return `${head}${v.title}　${SUBS_LABEL[v.subs] ?? v.subs}`;
  }

  async function reloadAi() {
    if (!inTauri) return;
    try {
      ai = await aiApi.getConfig();
    } catch (e) {
      toast(String(e));
    }
  }

  async function load() {
    if (!inTauri) return;
    await reloadAi();
    try {
      videos = await subApi.listVideos();
    } catch (e) {
      toast(String(e));
    }
    if (fileId == null || !videos.some(v => v.id === fileId)) fileId = videos[0]?.id ?? null;
    // 从别处带过来的视频（如番剧详情里的“AI 字幕”）
    if (ui.subVideo != null) {
      fileId = ui.subVideo;
      ui.subVideo = null;
    }
    // 之前还在跑的任务接着显示
    try {
      const all = await subApi.status();
      for (const j of all) jobInfo[j.id] = { step: j.step, progress: j.progress, message: j.message };
      const live = [...all].reverse().find(j => j.status === 'running' || j.status === 'queued');
      if (live) {
        jobId = live.id;
        running = true;
        applyStep(live.step, live.progress, live.message);
      }
    } catch {
      /* 没有任务 */
    }
  }

  function applyStep(step: string, p: number, message: string) {
    prog = p;
    msg = message;
    const idx = STEP_INDEX[step];
    if (idx === undefined) return;
    steps = SUB_STEPS.map((_, i) => (i < idx || (i === idx && p >= 1) ? 'ok' : i === idx ? 'run' : ''));
  }

  function onProgress(p: SubProgress) {
    jobInfo[p.job] = { step: p.step, progress: p.progress, message: p.message };
    if (p.job !== jobId) return;
    if (p.step === 'error') {
      steps = steps.map(s => (s === 'run' ? '' : s));
      msg = p.message;
      running = false;
      toast(p.message);
      return;
    }
    applyStep(p.step, p.progress, p.message);
    if (p.step === 'done' && p.progress >= 1) finish(p.job);
  }

  async function finish(id: number) {
    running = false;
    done = true;
    try {
      lines = await subApi.result(id);
      const info = (await subApi.status()).find(j => j.id === id);
      outputs = info?.outputs ?? [];
    } catch (e) {
      toast(String(e));
    }
    toast('字幕已生成并对齐，可以逐句修改，或直接在播放器中预览');
  }

  $effect(() => {
    // 从其它页面带过来的视频：页面已经打开时也要生效
    if (ui.subVideo != null) {
      fileId = ui.subVideo;
      ui.subVideo = null;
    }
  });

  $effect(() => {
    // 弹窗关闭后（可能刚改过 AI 服务设置）重新读取状态
    if (ui.modal === null) reloadAi();
  });

  $effect(() => {
    const id = fileId;
    est = null;
    if (id == null || !inTauri) return;
    subApi.estimate(id).then(e => (est = e)).catch(() => (est = null));
    overOk = false;
  });

  onMount(() => {
    load();
    let un: (() => void) | undefined;
    listenSubProgress(onProgress).then(f => (un = f));
    return () => un?.();
  });

  async function start() {
    if (!inTauri) return toast('请在桌面程序中使用');
    if (!selected) return toast('先选一个视频');
    if (!hasAi) return toast('请先配置 AI 服务');
    if (est?.over_limit && !overOk) {
      overOk = true;
      return toast('本月用量会超过上限，再点一次“开始生成”确认');
    }
    overOk = false;
    const options = {
      lang: lang === 'auto' ? null : lang,
      target,
      output,
      glossary: glossary.trim() || null,
    };
    steps = ['', '', '', '', '', ''];
    prog = 0;
    msg = '';
    lines = [];
    outputs = [];
    done = false;
    jobId = null;
    batchIds = [];
    batchViews = [];
    running = true;
    try {
      if (batch) {
        if (selected.series_id == null) throw new Error('这个视频不是番剧或剧集，不能整季批量');
        batchIds = await subApi.batch(selected.series_id, selected.season, options);
        const all = await subApi.status();
        batchViews = all.filter(j => batchIds.includes(j.id));
        toast(`已排队 ${batchIds.length} 集，依次处理`);
        running = false;
      } else {
        jobId = await subApi.start(selected.id, options);
      }
    } catch (e) {
      running = false;
      toast(String(e));
    }
  }

  async function cancel() {
    const ids = jobId != null ? [jobId, ...batchIds] : batchIds;
    for (const id of ids) {
      try {
        await subApi.cancel(id);
      } catch {
        /* 已经结束 */
      }
    }
    toast('已请求取消');
  }

  async function save() {
    if (jobId == null) return;
    try {
      outputs = await subApi.save(jobId, lines);
      toast('字幕已保存');
    } catch (e) {
      toast(String(e));
    }
  }

  function preview() {
    if (selected) ui.openItem(selected.id, 'video');
  }

  function drawTimeline(c: HTMLCanvasElement, ls: SubLine[], dur: number) {
    const dpr = window.devicePixelRatio || 1;
    const w = c.clientWidth;
    const h = 90;
    c.width = w * dpr;
    c.height = h * dpr;
    const g = c.getContext('2d');
    if (!g) return;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, w, h);
    const total = Math.max(dur, ls.length ? ls[ls.length - 1].end : 0, 1);
    const x = (s: number) => (s / total) * w;
    g.fillStyle = 'rgba(255,255,255,.06)';
    g.fillRect(0, 4, w, h * 0.6);
    const acc = getComputedStyle(document.documentElement).getPropertyValue('--mp-accent').trim() || '#7aa2ff';
    g.fillStyle = acc;
    g.globalAlpha = 0.85;
    for (const l of ls) g.fillRect(x(l.start), h * 0.2, Math.max(3, x(l.end) - x(l.start)), h * 0.4);
    g.globalAlpha = 1;
    g.fillStyle = '#ffffff80';
    g.font = '10px Consolas, monospace';
    const every = Math.ceil(total / 6);
    for (let s = 0; s <= total; s += every) g.fillText(tcLabel(s), x(s) + 3, h - 4);
  }

  $effect(() => {
    const c = canvas;
    const ls = lines;
    const dur = selected?.duration ?? 0;
    if (c) drawTimeline(c, ls, dur);
  });
</script>

<section class="lib on">
  <header>
    <div>
      <h2>AI 字幕</h2>
      <p>自动听写、断句、对齐时间轴，可选翻译成双语，适用于视频和番剧</p>
    </div>
  </header>

  {#if inTauri && !hasAi}
    <div class="note" style="margin-top:14px;display:flex;gap:12px;align-items:center;justify-content:space-between;flex-wrap:wrap">
      <span>还没有配置 AI 服务：字幕需要语音识别和翻译接口，填写一次即可。也可以改用本地模型（地址填 localhost）。</span>
      <button class="mp-btn ghost" onclick={() => ui.open('ai')}>去设置</button>
    </div>
  {/if}

  <div class="ai" style="margin-top:14px">
    <div class="card">
      <h5>任务设置</h5>
      <div class="frow">
        <span>文件</span>
        <select class="sel" style="flex:1;min-width:0" bind:value={fileId} disabled={running}>
          {#each videos as v (v.id)}
            <option value={v.id}>{label(v)}</option>
          {/each}
          {#if !videos.length}<option value={null}>（媒体库里还没有视频）</option>{/if}
        </select>
      </div>
      <div class="frow">
        <span>原声语言</span>
        <select class="sel" bind:value={lang} disabled={running}>
          <option value="auto">自动检测</option>
          <option value="ja">日语</option>
          <option value="en">英语</option>
          <option value="zh">中文</option>
        </select>
      </div>
      <div class="frow">
        <span>字幕语言</span>
        <select class="sel" bind:value={target} disabled={running}>
          <option value="both">简体中文 + 原文（双语）</option>
          <option value="zh">仅简体中文</option>
          <option value="src">仅原文听写</option>
        </select>
      </div>
      <div class="frow">
        <span>输出</span>
        <select class="sel" bind:value={output} disabled={running}>
          <option value="ass">外挂字幕 .ass（放在视频旁边）</option>
          <option value="srt">外挂字幕 .srt</option>
          <option value="mkv">封装进 MKV（不重新编码）</option>
        </select>
      </div>
      <div class="frow">
        <label style="display:flex;gap:8px;align-items:center">
          <input type="checkbox" class="mp-sw" bind:checked={batch} disabled={running} />
          整季批量处理缺字幕的集，并保持人名、术语一致
        </label>
      </div>
      <div class="frow" style="align-items:flex-start">
        <span>术语表</span>
        <textarea class="sel" rows="3" style="flex:1;min-width:0;resize:vertical" placeholder="每行一条：原文=译文" bind:value={glossary} disabled={running}></textarea>
      </div>
      <div class="note" style="margin:10px 0">{estText}</div>
      <div style="display:flex;gap:8px;flex-wrap:wrap">
        <button class="mp-btn" onclick={start} disabled={running || !hasAi || fileId == null}>{running ? '处理中…' : '开始生成'}</button>
        {#if running}<button class="mp-btn ghost" onclick={cancel}>取消</button>{/if}
        <button class="mp-btn ghost" onclick={preview} disabled={!done || !selected} style:opacity={done ? 1 : 0.4}>在播放器中预览</button>
      </div>
      <div class="note" style="margin-top:10px">
        密钥只存本机（Windows 凭据管理器），只把完成任务必需的音频和文字发到你在“AI 服务”里填写的地址。
      </div>
    </div>

    <div class="card">
      <h5>处理进度</h5>
      <div class="steps">
        {#each SUB_STEPS as s, i (s)}
          <span class="st {steps[i]}">{s}</span>
        {/each}
      </div>
      {#if msg}<div class="note" style="margin-bottom:10px">{msg}</div>{/if}
      {#if running}
        <div class="frow" style="margin-bottom:8px"><span>当前步骤</span><div style="flex:1;min-width:0;height:6px;border-radius:4px;background:rgba(255,255,255,.1);overflow:hidden"><div style="height:100%;width:{Math.round(prog * 100)}%;background:var(--mp-accent);transition:width .3s"></div></div></div>
      {/if}
      <canvas bind:this={canvas} style="width:100%;height:90px;display:block;border-radius:10px;background:rgba(255,255,255,.03)"></canvas>

      {#if batchViews.length}
        <div class="subt">
          {#each batchViews as j (j.id)}
            {@const info = jobInfo[j.id]}
            <div class="sl">
              <code>{info ? SUB_STEPS[STEP_INDEX[info.step] ?? 0] ?? info.step : '排队中'}</code>
              <span>{j.title}</span>
              <span>{info?.message ?? '排队中'}</span>
            </div>
          {/each}
        </div>
      {:else}
        <div class="subt">
          {#each lines as l, i (i)}
            <div class="sl">
              <code>{tcLabel(l.start)} → {tcLabel(l.end)}</code>
              <input bind:value={l.src} aria-label="原文" />
              <input class="zh" bind:value={l.zh} placeholder={target === 'src' ? '（不翻译）' : '译文'} aria-label="译文" />
            </div>
          {:else}
            <div class="empty">点“开始生成”，字幕会在这里一条条出现。生成后每一句都可以直接修改。</div>
          {/each}
        </div>
      {/if}

      {#if lines.length}
        <div style="margin-top:12px;display:flex;gap:10px;align-items:center;flex-wrap:wrap">
          <button class="mp-btn" onclick={save}>保存字幕</button>
          {#if outputs.length}<span class="note">已写出：{outputs.join('、')}</span>{/if}
        </div>
      {/if}
    </div>
  </div>
</section>
