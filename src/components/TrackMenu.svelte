<script lang="ts">
  // 歌曲右键菜单
  import { player } from '../lib/player.svelte';
  import { ui, toast } from '../lib/ui.svelte';
  import { lib } from '../lib/library.svelte';
  import { invoke, type Track } from '../lib/api';
  let { x, y, tracks, onclose }: { x: number; y: number; tracks: Track[]; onclose: () => void } = $props();
  let el: HTMLDivElement;
  let pos = $state({ left: 0, top: 0 });
  $effect(() => {
    if (!el) return;
    pos = { left: Math.max(8, Math.min(x, innerWidth - el.offsetWidth - 8)), top: Math.max(8, Math.min(y, innerHeight - el.offsetHeight - 8)) };
  });
  const t = $derived(tracks[0]);
  const acts: [string, () => void][] = $derived([
    ['播放', () => player.playList(tracks, 0)],
    ['下一首播放', () => player.playNext(tracks)],
    ['加入播放队列', () => player.enqueue(tracks)],
    ['加入歌单…', () => ui.open('playlist-add', tracks.map(x => x.id))],
    [t.favorite ? '取消收藏' : '收藏', () => lib.toggleFav(t.id).then(v => toast(v ? '已收藏' : '已取消收藏'))],
    ['转换格式…', () => { ui.convertPaths = tracks.map(x => x.path); ui.go('convert'); }],
    ['打开所在文件夹', () => invoke('show_in_folder', { path: t.path })],
    ['文件信息', () => ui.open('about', t)],
  ]);
</script>

<svelte:window onpointerdown={e => !el?.contains(e.target as Node) && onclose()} onkeydown={e => e.key === 'Escape' && onclose()} />
<div class="menu open" bind:this={el} style:left="{pos.left}px" style:top="{pos.top}px">
  <h6>{t.title}</h6>
  {#each acts as [label, f]}<button onclick={() => { f(); onclose(); }}>{label}</button>{/each}
</div>
