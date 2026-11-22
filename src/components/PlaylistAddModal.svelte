<script lang="ts">
  // 加入歌单 / 新建歌单 / 重命名歌单
  import Modal from './Modal.svelte';
  import { ui, toast } from '../lib/ui.svelte';
  import { lib } from '../lib/library.svelte';
  import { invoke } from '../lib/api';
  const arg = ui.modalArg as number[] | { rename: number; name: string } | null;
  const rename = arg && !Array.isArray(arg) ? arg : null;
  const files = Array.isArray(arg) ? arg : [];
  let name = $state(rename?.name ?? '');
  let input: HTMLInputElement;
  $effect(() => {
    setTimeout(() => input?.focus(), 80);
  });

  async function create() {
    const n = name.trim();
    if (!n) return toast('名字不能为空');
    if (rename) {
      await invoke('rename_playlist', { id: rename.rename, name: n });
      toast('已重命名');
    } else {
      const id = await invoke<number>('create_playlist', { name: n });
      if (files.length) await invoke('playlist_add', { id, files });
      toast(files.length ? `已新建「${n}」并加入 ${files.length} 首` : `已新建歌单「${n}」`);
      ui.playlistId = id;
      ui.go('music');
    }
    await lib.refreshPlaylists();
    ui.close();
  }
  async function add(id: number, pname: string) {
    const n = await invoke<number>('playlist_add', { id, files });
    toast(n ? `已加入「${pname}」` : `「${pname}」里已经有了`);
    await lib.refreshPlaylists();
    ui.close();
  }
</script>

<Modal title={rename ? '重命名歌单' : files.length ? `加入歌单（${files.length} 首）` : '新建歌单'} onclose={() => ui.close()}>
  {#if files.length}
    <div style="display:grid;gap:4px;margin:8px 0 14px">
      {#each lib.playlists as p (p.id)}
        <button class="mp-chip" style="text-align:left;border-radius:10px" onclick={() => add(p.id, p.name)}>{p.name} <small style="color:var(--mp-sub)">· {p.count} 首</small></button>
      {/each}
    </div>
    <p class="mp-desc">或者新建一个：</p>
  {/if}
  <input bind:this={input} class="tin" style="width:100%" maxlength="40" bind:value={name} placeholder="例如：深夜循环" onkeydown={e => e.key === 'Enter' && create()} />
  <div class="rowbtns">
    <button class="mp-btn" onclick={create}>{rename ? '确定' : '新建'}</button>
    <button class="mp-btn ghost" onclick={() => ui.close()}>取消</button>
  </div>
</Modal>
