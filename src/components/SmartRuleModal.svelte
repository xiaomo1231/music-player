<script lang="ts">
  // 自定义智能列表：用规则组合，比如“播放次数大于 20 且无损且最近 30 天听过”
  import Modal from './Modal.svelte';
  import { untrack } from 'svelte';
  import { customLists, FIELDS, matchRules, type Rule, type SmartDef } from '../lib/smart.svelte';
  import { lib } from '../lib/library.svelte';
  import { toast } from '../lib/ui.svelte';
  let { def, onclose }: { def: SmartDef | null; onclose: () => void } = $props();
  const init = untrack(() => def);
  let name = $state(init?.name ?? '');
  let limit = $state(init?.limit ?? 0);
  let rules = $state<Rule[]>(init?.rules ? structuredClone(init.rules) : [{ field: 'plays', op: '>', value: '20' }]);
  const kindOf = (f: Rule['field']) => FIELDS.find(x => x[0] === f)?.[2] ?? 'num';
  const count = $derived(lib.tracks.filter(t => matchRules(t, rules)).length);

  async function save() {
    if (!name.trim()) return toast('给列表起个名字');
    await customLists.save({ key: def?.key ?? 'c' + Date.now(), name: name.trim(), rules: $state.snapshot(rules), limit: limit || undefined });
    toast('已保存');
    onclose();
  }
  async function del() {
    if (def && confirm(`删除智能列表「${def.name}」？`)) {
      await customLists.remove(def.key);
      onclose();
    }
  }
</script>

<Modal title={def ? '编辑智能列表' : '新建智能列表'} {onclose} wide>
  <p class="mp-desc">满足下面全部条件的歌曲会自动出现在这个列表里，不用手动整理。当前匹配 <b>{count}</b> 首。</p>
  <input class="tin" style="width:100%;margin-bottom:10px" placeholder="列表名称，例如：最近单曲循环" bind:value={name} />
  {#each rules as r, i}
    <div class="frow" style="margin:6px 0">
      <select class="sel" bind:value={r.field} onchange={() => { const k = kindOf(r.field); r.op = k === 'text' ? 'has' : k === 'bool' ? '=' : '>'; r.value = k === 'bool' ? 'true' : ''; }}>
        {#each FIELDS as f}<option value={f[0]}>{f[1]}</option>{/each}
      </select>
      {#if kindOf(r.field) === 'num'}
        <select class="sel" bind:value={r.op}><option value=">">大于</option><option value="<">小于</option><option value="=">等于</option></select>
        <input class="tin" type="number" style="width:90px" bind:value={r.value} />
      {:else if kindOf(r.field) === 'bool'}
        <select class="sel" bind:value={r.value}><option value="true">是</option><option value="false">否</option></select>
      {:else}
        <input class="tin" style="flex:1" bind:value={r.value} placeholder="关键字" />
      {/if}
      <button class="mp-chip" onclick={() => rules.splice(i, 1)}>删除</button>
    </div>
  {/each}
  <div class="frow" style="margin-top:8px">
    <button class="mp-chip" onclick={() => rules.push({ field: 'lossless', op: '=', value: 'true' })}>＋ 添加条件</button>
    <span>最多</span><input class="tin" type="number" style="width:80px" bind:value={limit} /><span style="color:var(--mp-sub)">首（0 表示不限）</span>
  </div>
  <div class="rowbtns">
    <button class="mp-btn" onclick={save}>保存</button>
    {#if def}<button class="mp-btn ghost" onclick={del}>删除这个列表</button>{/if}
    <button class="mp-btn ghost" onclick={onclose}>取消</button>
  </div>
</Modal>
