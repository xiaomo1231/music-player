<script lang="ts">
  // 设置页“规则文件”：已导入的 .em 规则列表、导入、启用 / 停用、测试、编辑、导出、删除
  import { inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { confirmDlg } from '../../lib/items.svelte';
  import { em, emList, emSetEnabled, emDelete, emOpen, exportEm, pickEm, EM_KINDS, type EmRule, type EmKind } from '../../lib/em.svelte';
  import Modal from '../Modal.svelte';
  import ComicTest from './test/ComicTest.svelte';
  import SourceTest from './test/SourceTest.svelte';
  import CleanTest from './test/CleanTest.svelte';
  import OrganizeTest from './test/OrganizeTest.svelte';

  let rules = $state.raw<EmRule[]>([]);
  let filter = $state<'' | EmKind>('');
  let testing = $state.raw<EmRule | null>(null);

  async function load() {
    try {
      rules = await emList();
    } catch (e) {
      toast(String(e));
    }
  }

  $effect(() => {
    void em.version;
    void load();
  });

  const shown = $derived(filter ? rules.filter(r => r.kind === filter) : rules);
  const count = (k: EmKind) => rules.filter(r => r.kind === k).length;

  async function toggle(r: EmRule) {
    try {
      await emSetEnabled(r.id, !r.enabled);
      em.version++;
    } catch (e) {
      toast(String(e));
    }
  }

  async function remove(r: EmRule) {
    const ok = await confirmDlg('删除规则', `删除「${r.name}」。规则文件会一起删掉，已经用这个规则导出的文件不受影响。`, { ok: '删除', danger: true });
    if (!ok) return;
    try {
      await emDelete(r.id);
      em.version++;
      toast('已删除');
    } catch (e) {
      toast(String(e));
    }
  }

  async function edit(r: EmRule) {
    try {
      await emOpen(r.id);
      toast('改完保存后，下次使用时自动生效');
    } catch (e) {
      toast(String(e));
    }
  }
</script>

<div class="card em-card">
  <h5>规则文件</h5>
  <p class="note">
    用 .em 规则文件让软件按你的规则排版漫画、整理小说、整理文件，或接入下载源。双击 .em 文件、拖进窗口，或点“导入”都可以添加。
    规则只能做固定的几种事，不会运行程序；下载源规则只能访问它列出的网站，导入前会请你确认。只导入你信任的、来源有权获取内容的规则。
  </p>
  <div class="em-bar">
    <div class="seg">
      <button class:on={filter === ''} onclick={() => (filter = '')}>全部 {rules.length || ''}</button>
      {#each EM_KINDS as k (k.id)}
        <button class:on={filter === k.id} onclick={() => (filter = k.id)}>{k.name} {count(k.id) || ''}</button>
      {/each}
    </div>
    <button class="mp-btn" disabled={!inTauri} onclick={pickEm}>导入…</button>
  </div>
  {#if !shown.length}
    <p class="note empty">{rules.length ? '这一类还没有规则' : '还没有导入任何规则'}</p>
  {:else}
    <div class="em-list">
      {#each shown as r (r.id)}
        <div class="em-row" class:off={!r.enabled || !!r.problem}>
          <div class="em-main">
            <div class="em-name">
              <span class="badge2">{r.kind_label}</span>
              {r.name}
              {#if r.version}<small>v{r.version}</small>{/if}
            </div>
            {#if r.problem}
              <div class="em-problem">{r.problem}</div>
            {:else}
              <div class="em-sub">
                {r.note || r.id}{#if r.author} · {r.author}{/if}{#if r.hosts.length} · 访问 {r.hosts.join('、')}{/if}
              </div>
            {/if}
          </div>
          <div class="em-acts">
            <button class="mp-btn ghost" disabled={!r.enabled || !!r.problem} onclick={() => (testing = r)}>测试</button>
            <button class="mp-btn ghost" onclick={() => edit(r)}>编辑</button>
            <button class="mp-btn ghost" onclick={() => exportEm(r)}>导出</button>
            <button class="mp-btn ghost" onclick={() => remove(r)}>删除</button>
            <input type="checkbox" class="mp-sw" checked={r.enabled} title={r.enabled ? '停用' : '启用'} onchange={() => toggle(r)} />
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

{#if testing}
  <Modal title="测试：{testing.name}" wide onclose={() => (testing = null)}>
    {#if testing.kind === 'comic'}<ComicTest rule={testing} />
    {:else if testing.kind === 'source'}<SourceTest rule={testing} />
    {:else if testing.kind === 'clean'}<CleanTest rule={testing} />
    {:else}<OrganizeTest rule={testing} />{/if}
  </Modal>
{/if}

<style>
  .note {
    color: var(--mp-sub);
    font-size: 12.5px;
    line-height: 1.6;
    margin: 4px 0 8px;
  }
  .empty {
    text-align: center;
    padding: 14px 0;
  }
  .em-bar {
    display: flex;
    gap: 10px;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    margin-bottom: 8px;
  }
  .em-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .em-row {
    display: flex;
    gap: 12px;
    align-items: center;
    padding: 8px 10px;
    border-radius: 10px;
    background: rgba(255, 255, 255, 0.04);
  }
  .em-row.off .em-main {
    opacity: 0.6;
  }
  .em-main {
    flex: 1;
    min-width: 0;
  }
  .em-name {
    font-weight: 600;
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .em-name small {
    color: var(--mp-sub);
    font-weight: 400;
  }
  .em-sub,
  .em-problem {
    font-size: 12px;
    color: var(--mp-sub);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .em-problem {
    color: #ff8a8a;
    white-space: pre-line;
  }
  .em-acts {
    display: flex;
    gap: 6px;
    align-items: center;
    flex-shrink: 0;
  }
</style>
