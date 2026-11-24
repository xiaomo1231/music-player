<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { open } from '@tauri-apps/plugin-dialog';
  import { invoke, listen, inTauri } from '../lib/api';
  import { ui, toast } from '../lib/ui.svelte';
  import { lib } from '../lib/library.svelte';
  import { player } from '../lib/player.svelte';
  import {
    AFTER_OPTIONS,
    MEDIA_EXTS,
    VIDEO_EXTS,
    converterAccepts,
    converterPreset,
    extOf,
    parseConverterPreset,
    baseName,
    dirName,
    encoderLabel,
    isAudioPath,
    isVideoPath,
    loadAfter,
    saveAfter,
    type ConvertAfter,
    type ConvertJob,
    type ConvertProgress,
    type ConvertSettingsView,
    type HwStatus,
    type PresetInfo,
  } from '../lib/convert';
  import Queue from '../components/convert/Queue.svelte';
  import type { ConverterView, SettingDef } from '../lib/plugin.svelte';

  let presets = $state<PresetInfo[]>([]);
  let presetId = $state('mp3');
  let after = $state<ConvertAfter>(loadAfter());
  /** 待转换的文件（拖入、选择，或从播放栏 / 右键菜单带过来） */
  let pending = $state<string[]>([]);
  let jobs = $state<ConvertJob[]>([]);
  let speeds = $state<Record<number, number | null>>({});
  let hw = $state<HwStatus | null>(null);
  let cfg = $state<ConvertSettingsView | null>(null);
  let pluginAck = $state(false);
  let dragging = $state(false);
  let starting = $state(false);
  /** 已启用的插件提供的转换器 */
  let converters = $state<ConverterView[]>([]);
  /** 每个转换器的参数（键：插件/转换器） */
  let optValues = $state<Record<string, Record<string, unknown>>>({});

  const preset = $derived(presets.find(p => p.id === presetId));
  const plg = $derived(parseConverterPreset(presetId));
  const conv = $derived(plg ? converters.find(c => c.plugin === plg.plugin && c.id === plg.id) : undefined);
  const convOut = $derived(conv && plg ? conv.outputs.find(o => o.ext === plg.ext) : undefined);
  const convKey = (c: ConverterView) => `${c.plugin}/${c.id}`;
  /** 待转换的文件里这个转换器不支持的格式 */
  function unsupported(c: ConverterView): string[] {
    const bad = pending.filter(p => !converterAccepts(c.inputs, p)).map(p => extOf(p) || '无扩展名');
    return [...new Set(bad)];
  }
  const defaults = (c: ConverterView) => Object.fromEntries(c.options.map(o => [o.key, o.default ?? (o.type === 'bool' ? false : '')]));
  function optionsFor(c: ConverterView): Record<string, unknown> {
    return optValues[convKey(c)] ?? defaults(c);
  }
  // 选中转换器时准备好它的参数（保留之前填过的）
  $effect(() => {
    if (conv && !optValues[convKey(conv)]) optValues[convKey(conv)] = defaults(conv);
  });
  function optLabel(o: SettingDef['options'][number]): [string, string] {
    return typeof o === 'string' ? [o, o] : [o.value, o.label ?? o.value];
  }
  const afterOpt = $derived(AFTER_OPTIONS.find(a => a.value === after) ?? AFTER_OPTIONS[0]);
  const shownDir = $derived.by(() => {
    if (!cfg) return '';
    if (plg) return VIDEO_EXTS.includes(plg.ext) || ['gif', 'png', 'jpg', 'webp'].includes(plg.ext) ? cfg.out_dir_video : cfg.out_dir_audio;
    return preset?.kind === 'video' ? cfg.out_dir_video : cfg.out_dir_audio;
  });
  /** 这个预设实际会用到的编码器说明 */
  const hwNote = $derived.by(() => {
    if (!hw) return '';
    if (preset?.id === 'mp4_h264' || preset?.id === 'burn_sub') return `视频编码：${encoderLabel(hw.h264)}`;
    if (preset?.id === 'mp4_h265') return `视频编码：${encoderLabel(hw.hevc)}`;
    return '';
  });

  const unlisten: (() => void)[] = [];
  let dropOff: (() => void) | null = null;

  async function loadJobs() {
    if (!inTauri) return;
    try {
      jobs = await invoke<ConvertJob[]>('convert_list');
    } catch (e) {
      toast(String(e));
    }
  }

  // 多个任务同时结束时合并成一次刷新，避免几百条的列表被反复重读
  let reloadTimer: ReturnType<typeof setTimeout> | null = null;
  function reloadSoon() {
    if (reloadTimer) return;
    reloadTimer = setTimeout(() => {
      reloadTimer = null;
      void loadJobs();
    }, 400);
  }

  async function loadCfg() {
    if (!inTauri) return;
    try {
      cfg = await invoke<ConvertSettingsView>('convert_get_settings');
      pluginAck = cfg.plugin_ack;
    } catch (e) {
      toast(String(e));
    }
  }

  async function loadHw() {
    if (!inTauri) return;
    try {
      hw = await invoke<HwStatus>('convert_hw');
    } catch {
      // 硬件状态取不到时不影响转换
    }
  }

  async function loadConverters() {
    if (!inTauri) return;
    try {
      converters = await invoke<ConverterView[]>('convert_converters');
      // 选中的转换器被停用或卸载了：回到默认预设
      const p = parseConverterPreset(presetId);
      if (p && !converters.some(c => c.plugin === p.plugin && c.id === p.id && c.outputs.some(o => o.ext === p.ext))) presetId = 'mp3';
    } catch {
      converters = [];
    }
  }

  async function loadAll() {
    if (!inTauri) return;
    try {
      presets = await invoke<PresetInfo[]>('convert_presets');
    } catch (e) {
      toast(String(e));
    }
    await Promise.all([loadHw(), loadCfg(), loadJobs(), loadConverters()]);
  }

  /** 把拖入或选择的路径加入待转换列表（文件夹会展开成媒体文件） */
  async function addPaths(paths: string[]) {
    if (!inTauri) {
      toast('请在桌面程序中使用格式转换');
      return;
    }
    try {
      const files = await invoke<string[]>('convert_expand', { paths });
      const fresh = files.filter(f => !pending.includes(f));
      pending = [...pending, ...fresh];
      toast(fresh.length ? `已加入待转换：${fresh.length} 个文件` : '没有可以转换的音视频文件');
    } catch (e) {
      toast(String(e));
    }
  }

  /** 带过来的文件内置预设处理不了（比如加密格式）：自动选上一个能处理全部文件的转换插件。
   *  只是替用户点选插件，转换本身完全由插件完成 */
  function autoPick() {
    if (!pending.length || plg || pending.every(p => isAudioPath(p) || isVideoPath(p))) return;
    const c = converters.find(c => c.outputs.length && !unsupported(c).length);
    if (c) {
      presetId = converterPreset(c.plugin, c.id, c.outputs[0].ext);
      toast(`待转换 ${pending.length} 个文件，已选上转换插件：${c.name} → ${c.outputs[0].name}`);
    } else if (!(cfg?.plugin_enabled && cfg.plugin_file)) {
      toast('已装的插件里没有能处理这些格式的转换器，可以在“设置 → 插件”里安装');
    }
  }

  async function pickFiles() {
    if (!inTauri) return;
    const extra = [...new Set(converters.flatMap(c => c.inputs).filter(i => /^[a-z0-9]+$/.test(i) && i !== 'audio' && i !== 'video'))];
    const filters = [{ name: '音频和视频', extensions: [...new Set([...MEDIA_EXTS, ...extra])] }];
    if (converters.some(c => c.inputs.includes('*'))) filters.push({ name: '所有文件', extensions: ['*'] });
    const sel = await open({ multiple: true, filters });
    if (sel) await addPaths(Array.isArray(sel) ? sel : [sel]);
  }

  async function pickFolder() {
    if (!inTauri) return;
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel === 'string') await addPaths([sel]);
  }

  function setAfter(v: ConvertAfter) {
    after = v;
    saveAfter(v);
  }

  async function changeOutDir() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel !== 'string') return;
    try {
      await invoke('convert_set_settings', { outDir: sel, concurrency: null });
      await loadCfg();
      toast('输出目录已修改');
    } catch (e) {
      toast(String(e));
    }
  }

  async function resetOutDir() {
    try {
      await invoke('convert_set_settings', { outDir: '', concurrency: null });
      await loadCfg();
    } catch (e) {
      toast(String(e));
    }
  }

  async function setConcurrency(n: number) {
    try {
      await invoke('convert_set_settings', { outDir: null, concurrency: n });
      await loadCfg();
    } catch (e) {
      toast(String(e));
    }
  }

  async function startConvert() {
    if (!inTauri) return toast('请在桌面程序中使用格式转换');
    if (!pending.length) return toast('先拖入或选择要转换的文件');
    starting = true;
    try {
      const options = conv ? optionsFor(conv) : null;
      const ids = await invoke<number[]>('convert_add', { paths: pending, preset: presetId, after, outDir: null, options });
      pending = [];
      toast(`已加入队列：${ids.length} 个任务`);
      await loadJobs();
    } catch (e) {
      toast(String(e));
    } finally {
      starting = false;
    }
  }

  /** 加密格式插件：开启前必须先勾选确认 */
  async function setPlugin(el: HTMLInputElement) {
    const on = el.checked;
    if (on && !pluginAck) {
      el.checked = false;
      return toast('请先勾选下面的确认项');
    }
    try {
      await invoke('convert_set_plugin', { enabled: on, ack: pluginAck });
      await loadCfg();
    } catch (e) {
      el.checked = !on;
      toast(String(e));
    }
  }

  async function setAck(v: boolean) {
    pluginAck = v;
    if (!v && cfg?.plugin_enabled) {
      try {
        await invoke('convert_set_plugin', { enabled: false, ack: false });
        await loadCfg();
      } catch (e) {
        toast(String(e));
      }
    }
  }

  async function act(fn: () => Promise<unknown>, ok?: string) {
    try {
      await fn();
      if (ok) toast(ok);
    } catch (e) {
      toast(String(e));
    }
    await loadJobs();
  }

  const actions = {
    cancel: (id: number) => act(() => invoke('convert_cancel', { id })),
    retry: (id: number) => act(() => invoke('convert_retry', { id }), '已重新排队'),
    undo: (id: number) => act(() => invoke('convert_undo', { id }), '已撤销：原文件已从回收站恢复'),
    folder: (path: string) => {
      invoke('show_in_folder', { path }).catch(e => toast(String(e)));
    },
    play: (j: ConvertJob) => playJob(j),
    remove: (id: number) => act(() => invoke('convert_remove', { id }), '已删除记录'),
  };

  async function clearDone() {
    await act(() => invoke('convert_clear_done'), '已清除已结束的记录');
  }

  /** 把转换后的文件加入媒体库并播放（音频进播放器，视频进视频播放器） */
  async function playJob(j: ConvertJob) {
    if (!j.output) return;
    try {
      const added = await invoke<[number, string][]>('add_files', { paths: [j.output] });
      if (!added.length) return toast('文件没有加入媒体库，请用“在文件夹中显示”查看');
      const [id, kind] = added[0];
      await lib.refresh();
      if (isAudioPath(j.output)) {
        const t = lib.byId(id);
        if (t) {
          player.playOne(t);
          toast('正在播放转换后的文件');
        }
      } else {
        ui.openItem(id, kind);
      }
    } catch (e) {
      toast(String(e));
    }
  }

  async function onProgress(p: ConvertProgress) {
    speeds[p.id] = p.speed;
    const j = jobs.find(x => x.id === p.id);
    if (!j) {
      reloadSoon();
      return;
    }
    j.progress = p.progress;
    j.status = p.status;
    if (p.error) j.error = p.error;
    if (p.status === 'done' || p.status === 'failed' || p.status === 'canceled' || p.status === 'undone' || p.status === 'paused') {
      if (p.status === 'done') toast('转换完成：' + baseName(j.input));
      if (p.status === 'failed') toast('转换失败：' + (p.error ?? '未知错误'));
      reloadSoon();
    }
  }

  onMount(() => {
    const incoming = [...ui.convertPaths];
    ui.convertPaths = [];
    // 先读到转换插件列表，再收下带过来的文件，才能自动选插件
    loadAll().then(async () => {
      if (!incoming.length) return;
      await addPaths(incoming);
      autoPick();
    });
    if (inTauri) {
      getCurrentWindow()
        .onDragDropEvent(ev => {
          const p = ev.payload;
          if (p.type === 'enter' || p.type === 'over') dragging = true;
          else if (p.type === 'leave') dragging = false;
          else if (p.type === 'drop') {
            dragging = false;
            addPaths(p.paths);
          }
        })
        .then(off => (dropOff = off));
      listen<ConvertProgress>('convert://progress', p => onProgress(p)).then(off => unlisten.push(off));
      listen('convert://hw', () => loadHw()).then(off => unlisten.push(off));
      listen('plugin://changed', () => loadConverters()).then(off => unlisten.push(off));
    }
  });

  onDestroy(() => {
    dropOff?.();
    for (const off of unlisten) off();
    if (reloadTimer) clearTimeout(reloadTimer);
  });
</script>

<section class="lib on">
  <header>
    <div>
      <h2>格式转换</h2>
      <p>转完可以直接替换原文件或加入媒体库，也可以在播放界面一键转换当前曲目</p>
    </div>
  </header>

  <div class="cv" style="margin-top:14px">
    <div
      class="mp-drop"
      class:on={dragging}
      role="button"
      tabindex="0"
      onclick={pickFiles}
      onkeydown={e => {
        if (e.key === 'Enter' || e.key === ' ') pickFiles();
      }}
    >
      <b>把文件或文件夹拖到这里，或点击选择</b>
      内置 FFmpeg，支持音频、视频的转码与换封装，可批量排队{converters.length ? '；装好的转换插件支持的格式也可以拖进来' : ''}
      <div class="acts" style="justify-content:center;margin-top:12px">
        <button
          class="mp-btn ghost"
          onclick={e => {
            e.stopPropagation();
            pickFolder();
          }}>选择文件夹</button
        >
      </div>
    </div>

    {#if pending.length}
      <div class="pend">
        <div class="pend-head">
          <span>待转换 {pending.length} 个</span>
          <button class="mp-btn ghost" onclick={() => (pending = [])}>清空</button>
        </div>
        {#each pending as f, i (f)}
          <div class="pend-row">
            <span title={f}>{baseName(f)}</span>
            <small>{dirName(f)}</small>
            <button class="rm" onclick={() => (pending = pending.filter((_, k) => k !== i))} aria-label="移除">✕</button>
          </div>
        {/each}
      </div>
    {/if}

    <div class="frow">
      <span>转换为</span>
      <div class="chips" id="cpre">
        {#each presets as p (p.id)}
          <button class="mp-chip" class:on={presetId === p.id} title={p.note} onclick={() => (presetId = p.id)}>{p.name}</button>
        {/each}
      </div>
    </div>
    <div class="frow">
      <span>插件</span>
      {#if converters.length}
        <div class="chips">
          {#each converters as c (convKey(c))}
            {@const bad = unsupported(c)}
            {#each c.outputs as o (o.ext)}
              {@const id = converterPreset(c.plugin, c.id, o.ext)}
              <button
                class="mp-chip"
                class:on={presetId === id}
                disabled={bad.length > 0}
                title={bad.length ? `不支持 .${bad.join(' / .')} 文件` : `${c.plugin_name}：${c.note || c.inputs.join(' / ')}`}
                onclick={() => (presetId = id)}>{c.name} → {o.name}</button
              >
            {/each}
          {/each}
        </div>
      {:else}
        <span class="sub">还没有转换插件。装了提供格式转换的插件（设置 → 插件）后会出现在这里，可以处理任何它声明的格式。</span>
      {/if}
    </div>

    {#if conv && convOut}
      {@const vals = optionsFor(conv)}
      <div class="note">
        插件“{conv.plugin_name}”：{conv.name} → {convOut.name}（支持 {conv.inputs.join(' / ')}）{conv.note ? `。${conv.note}` : ''}{conv.exec
          ? '。这个转换器不报告进度，进度条会停在开头直到完成'
          : ''}
        {#if conv.needs_settings.length}<br />先在“设置 → 插件 → {conv.plugin_name}”里填写：{conv.needs_settings.join('、')}{/if}
      </div>
      {#each conv.options as o (o.key)}
        <div class="frow">
          <span title={o.help}>{o.label}</span>
          {#if o.type === 'bool'}
            <input type="checkbox" checked={Boolean(vals[o.key])} onchange={e => (vals[o.key] = e.currentTarget.checked)} />
          {:else if o.type === 'select'}
            <select class="sel" value={String(vals[o.key] ?? '')} onchange={e => (vals[o.key] = e.currentTarget.value)}>
              {#each o.options as op (optLabel(op)[0])}
                <option value={optLabel(op)[0]}>{optLabel(op)[1]}</option>
              {/each}
            </select>
          {:else if o.type === 'int'}
            <input class="tin" type="number" step="1" value={String(vals[o.key] ?? '')} onchange={e => (vals[o.key] = e.currentTarget.value)} />
          {:else}
            <input class="tin" value={String(vals[o.key] ?? '')} onchange={e => (vals[o.key] = e.currentTarget.value)} />
          {/if}
          {#if o.help}<span class="sub">{o.help}</span>{/if}
        </div>
      {/each}
    {:else if preset}
      <div class="note">{preset.note}{hwNote ? `　·　${hwNote}` : ''}</div>
    {/if}

    <div class="frow">
      <span>硬件编码</span>
      {#if !hw}
        <span class="sub">读取中…</span>
      {:else if !hw.checked}
        <span class="sub">正在检测显卡编码器（完成前使用软件编码）</span>
      {:else}
        <span class="sub">H.264：{encoderLabel(hw.h264)}　H.265：{encoderLabel(hw.hevc)}</span>
      {/if}
    </div>

    <div class="frow">
      <span>完成后</span>
      <div class="seg">
        {#each AFTER_OPTIONS as a (a.value)}
          <button class:on={after === a.value} onclick={() => setAfter(a.value)}>{a.label}</button>
        {/each}
      </div>
    </div>
    <div class="note">{afterOpt.note}</div>

    {#if cfg && (after === 'lib' || after === 'dir')}
      <div class="frow">
        <span>输出目录</span>
        <code class="pth">{shownDir}</code>
        <button class="mp-btn ghost" onclick={changeOutDir}>修改</button>
        {#if cfg.out_dir}
          <button class="mp-btn ghost" onclick={resetOutDir}>恢复默认</button>
        {/if}
      </div>
    {/if}

    {#if cfg}
      <div class="frow">
        <span>同时转换</span>
        <select class="sel" value={cfg.concurrency_auto ? '0' : String(cfg.concurrency)} onchange={e => setConcurrency(Number(e.currentTarget.value))}>
          <option value="0">自动（{cfg.concurrency_default} 个）</option>
          {#each Array.from({ length: cfg.concurrency_max }, (_, i) => i + 1) as n (n)}
            <option value={String(n)}>{n} 个</option>
          {/each}
        </select>
        <small style="color: var(--mp-sub)">按 CPU 核数自动；电脑性能好可以调高，最多 {cfg.concurrency_max} 个</small>
      </div>
    {/if}

    <div>
      <button class="mp-btn" disabled={starting || !pending.length} onclick={startConvert}>
        {starting ? '正在加入…' : `开始转换（${pending.length}）`}
      </button>
    </div>

    <Queue {jobs} {speeds} {actions} onclear={clearDone} onremoved={loadJobs} />

    <div class="mp-plugin">
      <input
        type="checkbox"
        class="mp-sw"
        aria-label="启用加密格式插件"
        checked={cfg?.plugin_enabled ?? false}
        onchange={e => setPlugin(e.currentTarget)}
      />
      <div>
        <b style="color:var(--mp-text)">加密格式解锁插件（可选，默认关闭）</b><br />
        用于 .qmc / .mgg / .mflac 等格式。仅限转换你本人已购买、拥有使用权的内容。
        <label class="ack">
          <input type="checkbox" checked={pluginAck} onchange={e => setAck(e.currentTarget.checked)} />
          我确认：仅用于本人已购买、拥有使用权的内容
        </label>
        <div class="sub">
          {cfg?.plugin_file ? `已找到插件：${cfg.plugin_file}` : `未找到插件：请把插件 .exe 放到 ${cfg?.plugin_dir ?? '应用数据目录的 plugins 文件夹'}`}
        </div>
      </div>
    </div>
  </div>
</section>

<style>
  .mp-drop.on {
    border-color: var(--mp-accent);
    color: var(--mp-text);
  }
  .pend {
    display: grid;
    gap: 6px;
    margin-top: 12px;
  }
  .pend-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 13px;
    color: var(--mp-sub);
  }
  .pend-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 2px 10px;
    align-items: center;
    padding: 8px 12px;
    border-radius: 10px;
    background: var(--mp-panel2);
    font-size: 13px;
  }
  .pend-row small {
    grid-column: 1;
    color: var(--mp-sub);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pend-row span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pend-row .rm {
    grid-row: 1 / span 2;
    grid-column: 2;
    border: 0;
    background: none;
    color: var(--mp-sub);
    cursor: pointer;
  }
  .pth {
    font-size: 12.5px;
    word-break: break-all;
    color: var(--mp-sub);
  }
  .sub {
    font-size: 12px;
    color: var(--mp-sub);
  }
  .ack {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-top: 8px;
    color: var(--mp-text);
    cursor: pointer;
  }
  .mp-plugin {
    margin-top: 18px;
  }
</style>
