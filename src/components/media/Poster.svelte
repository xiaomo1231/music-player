<script lang="ts">
  import type { Snippet } from 'svelte';
  import { fileUrl } from '../../lib/api';
  import { posterBg, thumbUrl } from '../../lib/media.svelte';

  let {
    poster = null,
    frame = null,
    thumb = null,
    name = '',
    cls = 'mp-poster',
    alt = false,
    children,
  }: {
    /** 海报图（绝对路径）：本地 poster.jpg 或手动指定的封面 */
    poster?: string | null;
    /** 竖版截帧（缩略图目录里的文件名） */
    frame?: string | null;
    /** 横版缩略图（截帧） */
    thumb?: string | null;
    name?: string;
    cls?: string;
    alt?: boolean;
    children?: Snippet;
  } = $props();

  const src = $derived(poster ? fileUrl(poster) : frame ? thumbUrl(frame) : '');
  const bg = $derived(src ? `url("${src}")` : posterBg(null, thumb, name, alt));

  // 海报不是竖版（CD 封面那种方图、横图）时不硬裁：底下铺一层模糊的，上面完整显示
  let ratio = $state(0);
  $effect(() => {
    const u = poster ? src : '';
    ratio = 0;
    if (!u || typeof Image === 'undefined') return;
    const im = new Image();
    im.onload = () => {
      if (u === (poster ? src : '')) ratio = im.naturalWidth / Math.max(1, im.naturalHeight);
    };
    im.src = u;
  });
  const fit = $derived(!!poster && ratio > 0 && (ratio < 0.55 || ratio > 0.8));
</script>

<div class={cls} class:art={!!poster} class:fit style:background-image={fit ? null : bg}>
  {#if fit}
    <i class="mp-fit-bg" style:background-image={bg}></i>
    <i class="mp-fit-img" style:background-image={bg}></i>
  {/if}
  {@render children?.()}
</div>
