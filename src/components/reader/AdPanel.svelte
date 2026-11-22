<script lang="ts">
  import type { AdHit, AdEstimate, AdAct } from '../../lib/reader';

  let {
    hits,
    candidates,
    busy,
    estimate,
    raw,
    scanned,
    act,
  }: {
    hits: AdHit[];
    candidates: AdHit[];
    busy: string;
    estimate: AdEstimate | null;
    raw: boolean;
    scanned: boolean;
    act: (a: AdAct) => void;
  } = $props();

  const applied = $derived(hits.filter(h => h.status === 'applied'));
  const pending = $derived(hits.filter(h => h.status === 'pending'));

  /** 找相似的结果默认全选 */
  let picked = $state<Set<number>>(new Set());
  $effect(() => {
    picked = new Set(candidates.map((_, i) => i));
  });

  function toggle(i: number) {
    const s = new Set(picked);
    if (s.has(i)) s.delete(i);
    else s.add(i);
    picked = s;
  }

  const pickedItems = $derived(candidates.filter((_, i) => picked.has(i)));

  function where(h: AdHit) {
    return `第 ${h.chapter + 1} 章 · 第 ${h.para + 1} 段`;
  }

  function short(s: string) {
    return s.replace(/\s+/g, ' ').slice(0, 26);
  }
</script>

<div>
  <h4>去广告</h4>
  <p>可以让程序扫描全书，也可以在正文里选中一段广告，标记后查找相似的内容。原文件不会被改动，随时可以撤销。</p>

  <div style="display:flex;gap:8px;flex-wrap:wrap">
    <button type="button" class="pri" disabled={!!busy} onclick={() => act({ k: 'scan' })}>{scanned ? '重新扫描' : '扫描全书'}</button>
    <button type="button" disabled={!!busy} onclick={() => act({ k: 'estimate' })}>AI 识别</button>
  </div>

  {#if busy}
    <p style="margin-top:10px">{busy}</p>
  {/if}

  {#if estimate}
    <div class="mp-adrule" style="margin-top:10px">
      预计用量约 {estimate.tokens} token，将发送 {estimate.samples} 段可疑内容（全书共 {estimate.paragraphs} 段，不会发送整本书）。
      <div style="display:flex;gap:8px;margin-top:6px">
        <button type="button" class="pri" onclick={() => act({ k: 'ai' })}>确认调用 AI</button>
        <button type="button" onclick={() => act({ k: 'dismiss' })}>取消</button>
      </div>
    </div>
  {/if}

  {#if candidates.length}
    <h4 style="margin-top:14px">找到 {candidates.length} 处相似内容</h4>
    {#each candidates as c, i (i)}
      <div class="mp-adhit" style="display:flex;gap:6px;align-items:flex-start">
        <input type="checkbox" checked={picked.has(i)} onchange={() => toggle(i)} aria-label="选择这一处" />
        <button type="button" onclick={() => act({ k: 'jump', hit: c })} style="border:0;padding:0;text-align:left;flex:1;background:none">
          {short(c.text)}…
          <span class="mp-adtag {c.confidence === 'hi' ? 'hi' : 'mid'}">{c.confidence === 'hi' ? '把握高' : '需确认'}</span>
          <small style="display:block">{where(c)}</small>
        </button>
      </div>
    {/each}
    <div style="display:flex;gap:8px;flex-wrap:wrap;margin-top:8px">
      <button type="button" class="pri" disabled={!pickedItems.length} onclick={() => act({ k: 'applyCandidates', items: pickedItems })}>
        清理选中的 {pickedItems.length} 处
      </button>
      <button type="button" onclick={() => act({ k: 'dismiss' })}>关闭结果</button>
    </div>
  {/if}

  {#if pending.length}
    <h4 style="margin-top:14px">待确认 {pending.length} 处</h4>
    <p>拿不准的内容不会自动删除，请逐条确认。</p>
    {#each pending as h (h.id)}
      <div class="mp-adhit" style="display:grid;gap:4px">
        <button type="button" onclick={() => act({ k: 'jump', hit: h })} style="border:0;padding:0;text-align:left;background:none">
          {short(h.text)}…
          <span class="mp-adtag mid">需确认</span>
          <small style="display:block">{h.rule} · {where(h)}</small>
        </button>
        <div style="display:flex;gap:6px">
          <button type="button" class="pri" onclick={() => act({ k: 'confirm', items: [h] })}>确认清理</button>
          <button type="button" onclick={() => act({ k: 'cancel', item: h })}>不是广告</button>
        </div>
      </div>
    {/each}
  {/if}

  {#if applied.length}
    <h4 style="margin-top:14px">已清理 {applied.length} 处</h4>
    {#each applied as h (h.id)}
      <div class="mp-adhit gone" style="display:grid;gap:4px">
        <button type="button" onclick={() => act({ k: 'jump', hit: h })} style="border:0;padding:0;text-align:left;background:none;color:inherit">
          {short(h.text)}…
          <small style="display:block">{h.rule} · {where(h)}</small>
        </button>
        <div>
          <button type="button" onclick={() => act({ k: 'cancel', item: h })}>撤销这一处</button>
        </div>
      </div>
    {/each}
    <div style="display:flex;gap:8px;flex-wrap:wrap;margin-top:8px">
      <button type="button" onclick={() => act({ k: 'raw' })}>{raw ? '隐藏已清理内容' : '对照原文'}</button>
      <button type="button" onclick={() => act({ k: 'revertAll' })}>全部撤销</button>
      <button type="button" onclick={() => act({ k: 'export' })}>导出干净的副本</button>
    </div>
    <p style="margin-top:10px">现在只是阅读时隐藏，原文件没有改动。</p>
  {/if}

  {#if !hits.length && !candidates.length && !busy}
    <p style="margin-top:10px">还没有发现广告。点上面的“扫描全书”，或者在正文里选中一段文字。</p>
  {/if}

</div>
