<script lang="ts">
  // 去广告规则管理：查看、停用、删除（内置规则只能停用；规则文件带来的规则只读，跟着规则文件停用 / 删除）
  import { invoke, inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import type { AdRule } from '../../lib/reader';
  import { confirmDlg } from '../../lib/items.svelte';
  let open = $state(false);
  let rules = $state<AdRule[]>([]);

  async function load() {
    if (inTauri) rules = await invoke<AdRule[]>('adclean_rules', { scope: null });
  }
  $effect(() => {
    if (open) load();
  });
  async function toggle(r: AdRule) {
    await invoke('adclean_rule_set_enabled', { id: r.id, enabled: !r.enabled });
    load();
  }
  async function del(r: AdRule) {
    if (!(await confirmDlg('删除规则', `删除这条去广告规则？${r.note || r.pattern}`, { ok: '删除', danger: true }))) return;
    await invoke('adclean_rule_delete', { id: r.id });
    toast('已删除规则');
    load();
  }
  const scopeText = (s: string) => (s === 'global' ? '全局' : '本书');
  const fromEm = (r: AdRule) => r.source.startsWith('em:');
  // 规则文件按书名 / 路径限定时，每本书各有一行：同一条只显示一次
  const shown = $derived(
    rules.filter((r, i) => !fromEm(r) || rules.findIndex(x => x.source === r.source && x.kind === r.kind && x.pattern === r.pattern) === i),
  );
  const sourceText = (r: AdRule) =>
    r.source === 'builtin' ? '内置' : r.source === 'ai' ? 'AI' : fromEm(r) ? '规则文件 · 在“设置 › 规则文件”里管理' : '自定义';
</script>

<div style="margin-top:14px">
  <button type="button" onclick={() => (open = !open)}>{open ? '收起规则' : '管理规则…'}</button>
  {#if open}
    {#if !rules.length}<p style="margin-top:8px">还没有规则。</p>{/if}
    {#each shown as r (r.id)}
      <div class="mp-adrule" style="margin-top:6px;opacity:{r.enabled ? 1 : 0.55}">
        <span style="word-break:break-all">{fromEm(r) ? r.pattern : r.note || r.pattern}</span>
        <small style="display:block;opacity:.7">{fromEm(r) ? r.note : scopeText(r.scope)} · {sourceText(r)}{r.hits ? ` · 命中 ${r.hits}` : ''}</small>
        {#if !fromEm(r)}
          <div style="display:flex;gap:6px;margin-top:4px">
            <button type="button" onclick={() => toggle(r)}>{r.enabled ? '停用' : '启用'}</button>
            {#if r.source !== 'builtin'}<button type="button" onclick={() => del(r)}>删除</button>{/if}
          </div>
        {/if}
      </div>
    {/each}
  {/if}
</div>
