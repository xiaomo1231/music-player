<script lang="ts">
  // 规则管理页“测试”面板（Organize）：输入文件名，看认出了什么、会整理到哪里
  import { inTauri } from '../../../lib/api';
  import { emTest, type EmRule } from '../../../lib/em.svelte';
  let { rule }: { rule: EmRule } = $props();

  interface OrgLine {
    line: string;
    matched: boolean;
    entry?: number;
    media?: string;
    media_label?: string;
    fields?: Record<string, string>;
    dst?: string;
  }
  interface OrgResult {
    root: string;
    items: OrgLine[];
  }

  const ROOT = 'D:\\媒体';
  const SAMPLE = [
    '[Nekomoe kissaten] 葬送的芙莉莲 - 05 [1080p].mkv',
    '[LoliHouse] 葬送的芙莉莲 第二季 - 03 [WebRip 1080p].mkv',
    'D:\\下载\\漫长的季节\\漫长的季节.S01E03.2160p.mp4',
    '随便一个视频.mp4',
  ].join('\n');

  const FIELD: Record<string, string> = {
    series: '作品名',
    season: '季',
    episode: '集',
    title: '标题',
    year: '年份',
    group: '字幕组',
    artist: '艺人',
    album: '专辑',
    track: '音轨',
    special: '特典',
  };

  let text = $state(SAMPLE);
  let root = $state('');
  let busy = $state(false);
  let err = $state('');
  let res = $state.raw<OrgResult | null>(null);
  const hit = $derived(res ? res.items.filter(i => i.matched).length : 0);

  async function run() {
    busy = true;
    err = '';
    try {
      const lines = text.split('\n').map(l => l.trim()).filter(Boolean);
      res = await emTest<OrgResult>(rule.id, { lines, root: root.trim() || null });
    } catch (e) {
      res = null;
      err = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<p class="note">输入文件名或完整路径，一行一个，看「{rule.name}」能不能认出来、会整理到哪里。只是预览，不会移动文件。</p>
<textarea class="tin box" rows="6" bind:value={text} spellcheck="false"></textarea>
<div class="bar">
  <input class="tin" style="flex:1" placeholder="整理到（不填按 {ROOT} 显示）" bind:value={root} />
  <button class="mp-btn" disabled={busy || !inTauri || !text.trim()} onclick={run}>{busy ? '正在识别…' : '运行'}</button>
</div>
{#if err}<p class="err">{err}</p>{/if}
{#if res}
  <p class="note">认出 {hit} / {res.items.length} 个。认不出的文件整理时交给软件自带的识别。</p>
  <div class="list">
    {#each res.items as it, i (i)}
      <div class="row" class:miss={!it.matched}>
        <div class="src">{it.line}</div>
        {#if it.matched}
          <div class="meta">
            <span class="badge2">第 {it.entry} 种 · {it.media_label}</span>
            {#each Object.entries(it.fields ?? {}) as [k, v] (k)}<span class="kv">{FIELD[k] ?? k}：{v}</span>{/each}
          </div>
          <div class="dst">→ {it.dst}</div>
        {:else}
          <div class="meta">未匹配</div>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  .note {
    color: var(--mp-sub);
    font-size: 12.5px;
    margin: 0 0 8px;
  }
  .box {
    width: 100%;
    resize: vertical;
    font-family: inherit;
    line-height: 1.6;
  }
  .bar {
    display: flex;
    gap: 8px;
    margin: 8px 0;
  }
  .err {
    color: #ff8a8a;
    font-size: 12.5px;
    white-space: pre-wrap;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 340px;
    overflow: auto;
  }
  .row {
    background: var(--mp-panel2);
    border-radius: 10px;
    padding: 8px 12px;
    font-size: 12.5px;
    line-height: 1.6;
  }
  .row.miss {
    opacity: 0.6;
  }
  .src {
    word-break: break-all;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 10px;
    color: var(--mp-sub);
    margin-top: 2px;
  }
  .dst {
    color: var(--mp-accent);
    word-break: break-all;
    margin-top: 2px;
  }
</style>
