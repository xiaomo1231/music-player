<script lang="ts">
  // 规则管理页“测试”面板（下载源）：按关键字搜索 → 第一条的分集 → 解析下载地址（不下载）
  import { emTest, type EmRule } from '../../../lib/em.svelte';
  let { rule }: { rule: EmRule } = $props();

  interface Req {
    method: string;
    url: string;
    status: number | null;
    error: string | null;
  }
  interface Target {
    type: string;
    label: string;
    url?: string;
    ext?: string;
    count?: number;
    first?: string;
    last?: string;
    referer?: string;
    first_title?: string;
    preview?: string;
  }
  interface Step {
    name: string;
    ok: boolean;
    error?: string;
    requests: Req[];
    pages: { page: string; hits: number }[];
    warns: string[];
    count?: number;
    items?: Record<string, string>[];
    work?: string;
    item?: string;
    target?: Target;
  }
  interface Report {
    query: string;
    ok: boolean;
    count: number;
    min_results: number;
    message?: string;
    steps: Step[];
  }

  let query = $state('');
  let running = $state(false);
  let report = $state.raw<Report | null>(null);
  let error = $state('');

  async function run() {
    if (running) return;
    running = true;
    error = '';
    report = null;
    try {
      report = await emTest<Report>(rule.id, { query: query.trim() });
    } catch (e) {
      error = String(e);
    } finally {
      running = false;
    }
  }

  const fieldText = (v: string) => (v.length > 120 ? v.slice(0, 120) + '…' : v);
</script>

<div class="frow">
  <input
    class="tin"
    style="flex:1"
    placeholder="搜索关键字（留空用规则里的 test.query）"
    autocomplete="off"
    bind:value={query}
    onkeydown={e => e.key === 'Enter' && run()}
  />
  <button class="mp-btn" onclick={run} disabled={running}>{running ? '运行中…' : '运行'}</button>
</div>
<p class="note">按规则联网搜索一次，再解析第一条结果的下载地址，不会真的下载。同一网站每秒最多请求一次，请耐心等。</p>

{#if error}<p class="bad">{error}</p>{/if}

{#if report}
  <div class="sum" class:fail={!report.ok}>
    {#if report.ok}通过：搜索“{report.query}”得到 {report.count} 条，下载地址解析成功{:else}没有通过{/if}
    {#if report.message}<div class="msg">{report.message}</div>{/if}
  </div>
  <div class="steps-list">
    {#each report.steps as s, i (i)}
      <div class="step">
        <div class="head">
          <span class="dot" class:ok={s.ok}></span>
          {i + 1}. {s.name}
          {#if s.count != null}<small>{s.count} 条</small>{/if}
          {#if s.work}<small>作品：{s.work}</small>{/if}
          {#if s.item}<small>{s.item}</small>{/if}
        </div>
        {#each s.requests as r, j (j)}
          <div class="req">
            <span class="m">{r.method}</span>
            <span class="code" class:bad={r.status == null || r.status >= 400}>{r.status ?? '—'}</span>
            <span class="url" title={r.url}>{r.url}</span>
          </div>
          {#if r.error && r.error !== s.error}<div class="bad small">{r.error}</div>{/if}
        {/each}
        {#if s.pages.length}
          <div class="sub">列表：{s.pages.map((p, k) => `第 ${k + 1} 页 ${p.hits} 条`).join('，')}</div>
        {/if}
        {#if s.items?.length}
          <div class="items">
            {#each s.items as it, k (k)}
              <div class="item">
                {#each Object.entries(it) as [key, val] (key)}
                  <div class="kv"><span class="k">{key}</span><span class="v" class:blank={!val}>{val ? fieldText(val) : '（空）'}</span></div>
                {/each}
              </div>
            {/each}
          </div>
        {/if}
        {#if s.target}
          {@const t = s.target}
          <div class="item">
            <div class="kv"><span class="k">下载方式</span><span class="v">{t.label}{t.ext ? `（${t.ext}）` : ''}</span></div>
            {#if t.url}<div class="kv"><span class="k">地址</span><span class="v">{t.url}</span></div>{/if}
            {#if t.type === 'pages'}
              <div class="kv"><span class="k">图片</span><span class="v">{t.count} 张</span></div>
              {#if t.first}<div class="kv"><span class="k">第一张</span><span class="v">{t.first}</span></div>{/if}
              {#if t.referer}<div class="kv"><span class="k">Referer</span><span class="v">{t.referer}</span></div>{/if}
            {/if}
            {#if t.type === 'chapters'}
              <div class="kv"><span class="k">章节</span><span class="v">{t.count} 章</span></div>
              {#if t.first_title}<div class="kv"><span class="k">第一章</span><span class="v">{t.first_title}</span></div>{/if}
              {#if t.preview}<div class="kv"><span class="k">正文</span><span class="v pre">{t.preview}</span></div>{/if}
            {/if}
          </div>
        {/if}
        {#each s.warns as w, k (k)}<div class="warn">{w}</div>{/each}
        {#if s.error}<div class="bad">{s.error}</div>{/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .note {
    color: var(--mp-sub);
    font-size: 12.5px;
    margin: 6px 0 10px;
  }
  .sum {
    padding: 8px 10px;
    border-radius: 8px;
    background: rgba(123, 224, 163, 0.1);
    color: #7be0a3;
    font-size: 13px;
    margin-bottom: 10px;
  }
  .sum.fail {
    background: rgba(255, 138, 138, 0.1);
    color: #ff8a8a;
  }
  .msg {
    color: var(--mp-sub);
    font-size: 12px;
    margin-top: 2px;
  }
  .steps-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-height: 60vh;
    overflow-y: auto;
  }
  .step {
    padding: 8px 10px;
    border-radius: 10px;
    background: rgba(255, 255, 255, 0.04);
    font-size: 12.5px;
    min-width: 0;
  }
  .head {
    font-weight: 600;
    display: flex;
    gap: 8px;
    align-items: center;
    margin-bottom: 4px;
  }
  .head small {
    color: var(--mp-sub);
    font-weight: 400;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #ff8a8a;
    flex-shrink: 0;
  }
  .dot.ok {
    background: #7be0a3;
  }
  .req {
    display: flex;
    gap: 6px;
    align-items: baseline;
    min-width: 0;
    color: var(--mp-sub);
  }
  .m {
    font-size: 11px;
    opacity: 0.7;
  }
  .code {
    color: #7be0a3;
  }
  .url {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .sub {
    color: var(--mp-sub);
    margin-top: 2px;
  }
  .items {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 6px;
  }
  .item {
    border-left: 2px solid var(--edge, rgba(255, 255, 255, 0.15));
    padding-left: 8px;
    margin-top: 6px;
  }
  .kv {
    display: flex;
    gap: 8px;
    min-width: 0;
  }
  .k {
    color: var(--mp-sub);
    flex-shrink: 0;
    min-width: 64px;
  }
  .v {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .v.blank {
    color: var(--mp-sub);
    opacity: 0.6;
  }
  .v.pre {
    white-space: pre-line;
  }
  .warn {
    color: #f0c674;
    margin-top: 4px;
  }
  .bad {
    color: #ff8a8a;
    margin-top: 4px;
    overflow-wrap: anywhere;
  }
  .code.bad {
    margin: 0;
  }
  .small {
    font-size: 12px;
  }
</style>
