<script lang="ts">
  // AI 服务设置：接口地址、密钥（存在 Windows 凭据管理器）、三个模型、每月上限、用量、测试连接
  import Modal from './Modal.svelte';
  import { ui, toast } from '../lib/ui.svelte';
  import { inTauri, invoke } from '../lib/api';
  import { aiApi, localOcrApi, type AiConfig, type AiUsage, type LocalOcrStatus, type LocalOcrProgress } from '../lib/aisub';
  import { onDestroy } from 'svelte';

  const PRESETS: { id: string; name: string; url: string; hint: string }[] = [
    { id: 'openai', name: 'OpenAI', url: 'https://api.openai.com/v1', hint: '官方接口，按量计费' },
    { id: 'azure', name: 'Azure 兼容', url: 'https://your-resource.openai.azure.com/openai/v1', hint: '把 your-resource 换成你的资源名' },
    { id: 'ollama', name: '本地 Ollama', url: 'http://localhost:11434/v1', hint: '免费、离线，需要先在本机运行 Ollama' },
    { id: 'custom', name: '自定义', url: '', hint: '任何兼容 OpenAI 接口格式的服务' },
  ];

  let loaded = $state(false);
  let hasKey = $state(false);
  let usage = $state<AiUsage | null>(null);
  let preset = $state('custom');
  let baseUrl = $state('');
  let key = $state('');
  let asrModel = $state('');
  let chatModel = $state('');
  let visionModel = $state('');
  let limit = $state(0);
  let ocrEngine = $state<'cloud' | 'local'>('cloud');
  let classifyMedia = $state(true);
  let classifying = $state(false);
  let local = $state<LocalOcrStatus | null>(null);
  let dl = $state<LocalOcrProgress | null>(null);
  let localMsg = $state('');
  let localBusy = $state(false);
  let testing = $state(false);
  let testResult = $state('');
  let saved = $state('');

  const form = $derived(JSON.stringify([baseUrl, asrModel, chatModel, visionModel, limit, ocrEngine, classifyMedia]));
  const dirty = $derived(loaded && form !== saved);

  function presetOf(url: string): string {
    return PRESETS.find(p => p.id !== 'custom' && p.url === url)?.id ?? 'custom';
  }

  function applyPreset(id: string) {
    preset = id;
    const p = PRESETS.find(x => x.id === id);
    if (p && p.id !== 'custom') baseUrl = p.url;
  }

  async function load() {
    if (!inTauri) {
      loaded = true;
      return;
    }
    try {
      const st = await aiApi.getConfig();
      const c: AiConfig = st.config;
      baseUrl = c.base_url;
      asrModel = c.asr_model;
      chatModel = c.chat_model;
      visionModel = c.vision_model;
      limit = c.monthly_token_limit;
      ocrEngine = c.ocr_engine === 'local' ? 'local' : 'cloud';
      classifyMedia = c.classify_media !== false;
      hasKey = st.has_key;
      usage = st.usage;
      preset = presetOf(c.base_url);
      saved = form;
      loaded = true;
    } catch (e) {
      toast(String(e));
      loaded = true;
    }
  }

  async function save(clearKey = false) {
    const config: AiConfig = {
      base_url: baseUrl.trim(),
      asr_model: asrModel.trim(),
      chat_model: chatModel.trim(),
      vision_model: visionModel.trim(),
      monthly_token_limit: Math.max(0, Math.round(Number(limit) || 0)),
      ocr_engine: ocrEngine,
      classify_media: classifyMedia,
    };
    // 密钥留空表示不改；清除时传空串
    const k = clearKey ? '' : key.trim() ? key.trim() : null;
    try {
      await aiApi.saveConfig(config, k);
      key = '';
      if (clearKey) hasKey = false;
      else if (k) hasKey = true;
      saved = form;
      toast(clearKey ? '已清除保存的密钥' : '已保存');
      return true;
    } catch (e) {
      toast(String(e));
      return false;
    }
  }

  async function test() {
    if (!inTauri) return toast('请在桌面程序中使用');
    if (dirty || key.trim()) {
      // 测试的是已保存的设置，所以先保存
      const ok = await save();
      if (!ok) return;
    }
    testing = true;
    testResult = '正在测试…';
    try {
      testResult = '✓ ' + (await aiApi.test());
    } catch (e) {
      testResult = '连接失败：' + String(e);
    } finally {
      testing = false;
    }
  }

  function fmtNum(n: number): string {
    return n.toLocaleString();
  }

  // ---------- 本地文字识别 ----------
  let unlisten: (() => void) | null = null;

  async function refreshLocal() {
    if (!inTauri) return;
    try {
      local = await localOcrApi.status();
    } catch (e) {
      localMsg = String(e);
    }
  }

  async function startDownload() {
    localMsg = '';
    if (local) local.downloading = true;
    try {
      const done = await localOcrApi.download();
      localMsg = done ? '✓ 下载完成，可以用本地识别了' : '已暂停，下次点“继续下载”会接着下';
    } catch (e) {
      localMsg = String(e);
    } finally {
      dl = null;
      await refreshLocal();
    }
  }

  async function removeLocal() {
    try {
      await localOcrApi.remove();
      localMsg = '已删除模型文件';
    } catch (e) {
      localMsg = String(e);
    }
    await refreshLocal();
  }

  async function testLocal() {
    localBusy = true;
    localMsg = '正在启动本地识别，第一次需要十几秒…';
    try {
      localMsg = '✓ ' + (await localOcrApi.test());
    } catch (e) {
      localMsg = '测试失败：' + String(e);
    } finally {
      localBusy = false;
      await refreshLocal();
    }
  }

  // 立即把规则拿不准的视频交给 AI 识别
  async function classifyNow() {
    if (!inTauri) return toast('请在桌面程序中使用');
    if ((dirty || key.trim()) && !(await save())) return;
    classifying = true;
    try {
      const n = await invoke<number>('ai_classify_now');
      toast(n ? `AI 重新识别了 ${n} 个视频` : '没有需要 AI 识别的视频');
    } catch (e) {
      toast(String(e));
    } finally {
      classifying = false;
    }
  }

  function mb(n: number): string {
    return (n / 1048576).toFixed(0) + ' MB';
  }

  if (inTauri) {
    localOcrApi.onProgress(p => (dl = p)).then(f => (unlisten = f));
    refreshLocal();
  }
  onDestroy(() => unlisten?.());

  load();
</script>

<Modal title="AI 服务" onclose={() => ui.close()}>
  <p class="mp-desc">AI 字幕和漫画翻译需要语音识别和大语言模型。填一次，全局可用；也可以改用本地模型（免费、离线，但更慢，需要较好的显卡）。</p>

  <label class="mp-opt">
    <div>服务类型<small>{PRESETS.find(p => p.id === preset)?.hint ?? ''}</small></div>
    <select value={preset} onchange={e => applyPreset((e.currentTarget as HTMLSelectElement).value)}>
      {#each PRESETS as p (p.id)}
        <option value={p.id}>{p.name}</option>
      {/each}
    </select>
  </label>

  <label class="mp-opt">
    <div>接口地址<small>兼容 OpenAI 接口格式（/chat/completions、/audio/transcriptions）</small></div>
    <input bind:value={baseUrl} placeholder="https://api.example.com/v1" style="flex:1.4;min-width:0" />
  </label>

  <label class="mp-opt">
    <div>
      API 密钥
      <small>{hasKey ? '已保存在 Windows 凭据管理器，留空表示不修改' : '密钥只存本机，不写进数据库'}</small>
    </div>
    <input type="password" bind:value={key} placeholder={hasKey ? '••••••（已保存）' : 'sk-…'} autocomplete="off" style="flex:1.4;min-width:0" />
  </label>
  {#if hasKey}
    <div style="text-align:right;margin:-4px 0 8px">
      <button class="mp-btn ghost" style="font-size:12px;padding:3px 10px" onclick={() => save(true)}>清除已保存的密钥</button>
    </div>
  {/if}

  <label class="mp-opt">
    <div>语音识别模型<small>用于 AI 字幕，例如 whisper-1</small></div>
    <input bind:value={asrModel} style="flex:1.4;min-width:0" />
  </label>
  <label class="mp-opt">
    <div>翻译模型<small>用于字幕翻译和漫画翻译</small></div>
    <input bind:value={chatModel} style="flex:1.4;min-width:0" />
  </label>
  <label class="mp-opt">
    <div>图像识别模型<small>用于识别漫画里的文字，需要支持图片输入</small></div>
    <input bind:value={visionModel} style="flex:1.4;min-width:0" />
  </label>
  <label class="mp-opt">
    <div>
      AI 辅助识别媒体库
      <small>扫描后把规则拿不准的视频（是番剧、电视剧还是电影，作品名、第几集）交给翻译模型判断。只发文件夹名和文件名，同一个文件只问一次</small>
    </div>
    <input type="checkbox" bind:checked={classifyMedia} />
  </label>
  {#if classifyMedia}
    <div style="text-align:right;margin:-4px 0 8px">
      <button class="mp-btn ghost" style="font-size:12px;padding:3px 10px" onclick={classifyNow} disabled={classifying}>{classifying ? '正在识别…' : '现在识别一遍'}</button>
    </div>
  {/if}
  <label class="mp-opt">
    <div>
      漫画文字识别
      <small>{ocrEngine === 'local' ? '本机运行 Xiaomi-OCR-0，图片不上传，免费；有显卡会自动用显卡' : '把整页图片发给上面的图像识别模型'}</small>
    </div>
    <select bind:value={ocrEngine} style="flex:1.4;min-width:0">
      <option value="cloud">云端图像识别模型</option>
      <option value="local">本地识别（Xiaomi-OCR-0）</option>
    </select>
  </label>
  {#if ocrEngine === 'local' && inTauri}
    <div class="mp-opt" style="display:block">
      {#if !local}
        <small>正在读取本地识别状态…</small>
      {:else if !local.runtime}
        <small style="color:var(--mp-warn, #e5a00d)">没有找到本地识别程序（llama-server.exe，安装包自带），请重新安装播放器。</small>
      {:else if local.installed}
        <small style="display:block;line-height:1.7">识别模型已下载（{mb(local.size)}）{local.running ? '，正在运行' : ''}。</small>
        <div style="display:flex;gap:8px;margin-top:6px;flex-wrap:wrap">
          <button class="mp-btn ghost" style="font-size:12px;padding:3px 10px" onclick={testLocal} disabled={localBusy}>测试本地识别</button>
          <button class="mp-btn ghost" style="font-size:12px;padding:3px 10px" onclick={removeLocal} disabled={localBusy}>删除模型</button>
        </div>
      {:else}
        <small style="display:block;line-height:1.7">
          第一次使用需要下载识别模型（约 {mb(local.size)}），只下载一次。{local.got > 0 && !local.downloading ? `已下载 ${mb(local.got)}。` : ''}
        </small>
        {#if local.downloading}
          {@const got = dl?.got ?? local.got}
          {@const total = dl?.total || local.size}
          <div style="height:6px;border-radius:3px;background:var(--mp-line, rgba(127,127,127,.25));margin:8px 0 4px;overflow:hidden">
            <div style="height:100%;width:{Math.min(100, (got / total) * 100).toFixed(1)}%;background:var(--mp-accent, #7aa2ff)"></div>
          </div>
          <small>{dl?.message ?? '正在连接'} {mb(got)} / {mb(total)}{dl?.speed ? `，${(dl.speed / 1048576).toFixed(1)} MB/s` : ''}</small>
          <div style="margin-top:6px"><button class="mp-btn ghost" style="font-size:12px;padding:3px 10px" onclick={() => localOcrApi.cancel()}>暂停</button></div>
        {:else}
          <div style="margin-top:6px"><button class="mp-btn" style="font-size:12px;padding:3px 12px" onclick={startDownload}>{local.got > 0 ? '继续下载' : '下载识别模型'}</button></div>
        {/if}
      {/if}
      {#if localMsg}<small style="display:block;margin-top:6px;word-break:break-all">{localMsg}</small>{/if}
    </div>
  {/if}
  <label class="mp-opt">
    <div>每月 token 上限<small>超过上限前会先提醒，0 表示不限</small></div>
    <input type="number" min="0" step="10000" bind:value={limit} style="flex:1.4;min-width:0" />
  </label>

  <div class="mp-opt" style="display:block">
    <small style="display:block;line-height:1.7">
      本月用量：
      {#if usage}
        {usage.month}，输入 {fmtNum(usage.prompt_tokens)} tokens，输出 {fmtNum(usage.completion_tokens)} tokens，
        语音 {Math.round(usage.audio_seconds / 60)} 分钟，共 {fmtNum(usage.requests)} 次请求
      {:else}
        暂无记录
      {/if}
    </small>
    <small style="display:block;line-height:1.7;margin-top:6px">
      隐私说明：只会把完成任务必需的内容（视频的音轨、漫画页面图片、待翻译的文字）发送到你填写的服务地址，不经过任何第三方服务器；密钥用 Windows 凭据管理器保存在本机。
    </small>
  </div>

  <div style="margin-top:14px;display:flex;gap:10px;align-items:center;flex-wrap:wrap">
    <button class="mp-btn" onclick={() => save()} disabled={!loaded}>保存</button>
    <button class="mp-btn ghost" onclick={test} disabled={testing || !loaded}>{dirty ? '保存并测试连接' : '测试连接'}</button>
    <span style="font-size:12.5px;color:var(--mp-sub);word-break:break-all">{testResult}</span>
  </div>
</Modal>
