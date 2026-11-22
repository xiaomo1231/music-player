<script lang="ts">
  // AI 服务设置：接口地址、密钥（存在 Windows 凭据管理器）、三个模型、每月上限、用量、测试连接
  import Modal from './Modal.svelte';
  import { ui, toast } from '../lib/ui.svelte';
  import { inTauri } from '../lib/api';
  import { aiApi, type AiConfig, type AiUsage } from '../lib/aisub';

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
  let testing = $state(false);
  let testResult = $state('');
  let saved = $state('');

  const form = $derived(JSON.stringify([baseUrl, asrModel, chatModel, visionModel, limit]));
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
