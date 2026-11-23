<script lang="ts">
  // 规则管理页“测试”面板（Clean）：贴一段文字，看排版、去广告之后的样子
  import { inTauri } from '../../../lib/api';
  import { emTest, type EmRule } from '../../../lib/em.svelte';
  let { rule }: { rule: EmRule } = $props();

  interface CleanResult {
    output: string;
    removed: string[];
    replaced: number;
    chapters: string[];
  }

  const SAMPLE = [
    '第一章 雾港',
    '雾是从后半夜开始涨起来的,先是漫过防波堤,',
    '再漫过码头上那一排旧缆桩...',
    '他看了一眼表,三点十四分.',
    '本章完',
    '',
    '第二章 灯塔',
    '“灯还亮着吗?”她问。（本章未完，请翻页）',
    '网址 www.example.com 记得收藏,圆周率是3.14。',
  ].join('\n');

  let text = $state(SAMPLE);
  let busy = $state(false);
  let err = $state('');
  let res = $state.raw<CleanResult | null>(null);

  async function run() {
    busy = true;
    err = '';
    try {
      res = await emTest<CleanResult>(rule.id, { text });
    } catch (e) {
      res = null;
      err = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<p class="note">贴一段小说原文，看「{rule.name}」处理后的样子。只是预览，不会改任何文件。</p>
<textarea class="tin box" rows="9" bind:value={text} spellcheck="false"></textarea>
<div class="bar">
  <button class="mp-btn" disabled={busy || !inTauri || !text.trim()} onclick={run}>{busy ? '正在处理…' : '运行'}</button>
  <button class="mp-btn ghost" disabled={busy} onclick={() => (text = SAMPLE)}>恢复示例</button>
</div>
{#if err}<p class="err">{err}</p>{/if}
{#if res}
  <div class="cols">
    <div>
      <h6>处理后</h6>
      <pre class="out">{res.output || '（全部被删掉了）'}</pre>
    </div>
    <div>
      <h6>改了什么</h6>
      <ul class="chg">
        <li>替换 {res.replaced} 处</li>
        <li>
          分出 {res.chapters.length} 章{#if res.chapters.length}：{res.chapters.join('、')}{/if}
        </li>
        <li>
          删除 {res.removed.length} 处广告
          {#if res.removed.length}
            <ul>
              {#each res.removed as r, i (i)}<li class="rm">{r}</li>{/each}
            </ul>
          {/if}
        </li>
      </ul>
    </div>
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
  .cols {
    display: grid;
    grid-template-columns: 3fr 2fr;
    gap: 14px;
    margin-top: 6px;
  }
  h6 {
    margin: 0 0 6px;
    font-size: 12.5px;
    color: var(--mp-sub);
    font-weight: 500;
  }
  .out {
    margin: 0;
    max-height: 320px;
    overflow: auto;
    white-space: pre-wrap;
    word-break: break-all;
    font-family: inherit;
    font-size: 13px;
    line-height: 1.7;
    background: var(--mp-panel2);
    border-radius: 10px;
    padding: 10px 12px;
  }
  .chg {
    margin: 0;
    padding-left: 18px;
    font-size: 12.5px;
    line-height: 1.8;
  }
  .rm {
    text-decoration: line-through;
    color: var(--mp-sub);
    word-break: break-all;
  }
</style>
