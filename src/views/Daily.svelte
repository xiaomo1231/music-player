<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../lib/Icon.svelte';
  import Cover from '../components/Cover.svelte';
  import DailyBanner from '../components/daily/DailyBanner.svelte';
  import DailyStats from '../components/daily/DailyStats.svelte';
  import { lib } from '../lib/library.svelte';
  import { player } from '../lib/player.svelte';
  import { toast, ui } from '../lib/ui.svelte';
  import { inTauri, type Track } from '../lib/api';
  import { daily, dayLabel, SOURCE, type DailyDoc, type DailyHistory, type DailyItem, type Feedback } from '../lib/daily';

  let doc = $state<DailyDoc | null>(null);
  let loading = $state(false);
  let busy = $state(false);
  let aiBusy = $state(false);
  /** 回看的以前某一天；null 表示今天 */
  let pastDay = $state<string | null>(null);
  let history = $state<DailyHistory[]>([]);
  let showHistory = $state(false);
  let menuFor = $state<number | null>(null);
  let liked = $state<number[]>([]);

  /** 推荐里的歌在库里找到的部分（按推荐顺序） */
  const covers: Track[] = $derived(doc ? doc.tracks.map(i => lib.byId(i.id)).filter((t): t is Track => !!t) : []);
  const smallLib = $derived(!!doc && doc.stats.library_tracks < 10);

  function todayDay(): string {
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
  }

  async function load(day?: string) {
    if (!inTauri) return;
    loading = true;
    try {
      const d = await daily.get(day);
      doc = d;
      pastDay = day && day !== todayDay() ? day : null;
      liked = [];
    } catch (e) {
      toast(String(e));
    }
    loading = false;
  }

  onMount(() => {
    load();
    if (inTauri) daily.history().then(h => (history = h)).catch(() => {});
  });

  async function regen() {
    if (busy || !doc) return;
    busy = true;
    try {
      doc = await daily.regenerate();
      pastDay = null;
      liked = [];
      toast('换了一批（昨天和这一批推过的不会再出现）');
    } catch (e) {
      toast(String(e));
    }
    busy = false;
  }

  /** 从 id 那首开始播放整个推荐（只包含库里找得到的歌） */
  function playFrom(id?: number) {
    const list = covers;
    if (!list.length) return toast('没有可播放的歌');
    const i = id == null ? 0 : Math.max(0, list.findIndex(t => t.id === id));
    player.playList(list, i, 'daily');
  }

  async function save() {
    const cur = doc;
    if (!cur) return;
    try {
      await daily.savePlaylist(cur.day, '');
      await lib.refreshPlaylists();
      toast('已存为歌单，在左侧歌单里');
    } catch (e) {
      toast(String(e));
    }
  }

  async function aiEnhance() {
    const cur = doc;
    if (!cur || aiBusy) return;
    aiBusy = true;
    try {
      doc = await daily.aiEnhance(cur.day);
      toast('AI 点评已更新');
    } catch (e) {
      toast(String(e));
    }
    aiBusy = false;
  }

  async function feedback(item: DailyItem, kind: Feedback) {
    menuFor = null;
    const cur = doc;
    if (!cur) return;
    const artist = lib.byId(item.id)?.artist ?? '';
    try {
      await daily.feedback(item.id, kind);
    } catch (e) {
      toast(String(e));
      return;
    }
    // “不喜欢这个艺人”：这位艺人的歌都从这一批里去掉
    const keep = (x: DailyItem) => (kind === 'not_artist' && artist ? (lib.byId(x.id)?.artist ?? '') !== artist : x.id !== item.id);
    doc = { ...cur, tracks: cur.tracks.filter(keep) };
    const msg: Record<Feedback, string> = {
      not_song: '已记下：这首之后不再推',
      not_artist: '已记下：这位艺人之后少推',
      not_today: '今天不再推这首',
      like: '已记下：之后多推这类',
    };
    toast(msg[kind]);
  }

  async function like(item: DailyItem) {
    if (liked.includes(item.id)) return;
    liked = [...liked, item.id];
    try {
      await daily.feedback(item.id, 'like');
      toast('已记下：之后多推这类');
    } catch (e) {
      toast(String(e));
    }
  }
</script>

<section class="lib on">
  <header>
    <div>
      <h2>每日推荐</h2>
      <p>根据你的播放记录，从本地库里挑一份今天的歌单</p>
    </div>
  </header>

  {#if !inTauri}
    <div class="empty">在桌面程序里打开才能看到今日推荐</div>
  {:else if loading && !doc}
    <div class="empty">正在生成今日推荐…</div>
  {:else if doc}
    {#if doc.tracks.length}
      <DailyBanner {doc} {covers} past={!!pastDay} {busy} onplay={() => playFrom()} onregen={regen} onsave={save} />
    {:else}
      <div class="banner" style="min-height:0;grid-template-columns:1fr">
        <div>
          <h2>{doc.title}</h2>
          <p>{doc.summary}</p>
          {#if smallLib}
            <div class="acts"><button class="mp-btn" onclick={() => ui.open('scan')}>去扫描音乐</button></div>
          {/if}
        </div>
      </div>
    {/if}

    {#if pastDay}
      <div class="note" style="margin-top:12px">正在回看 {dayLabel(doc.day)} 的推荐</div>
    {/if}

    <DailyStats stats={doc.stats} />

    {#if doc.tracks.length}
      <div class="frow" style="margin:14px 0 10px">
        <div class="seg">
          <button class="on">只用已有的歌</button>
          <button disabled title="需要先在“自动获取”里添加音乐来源">加入新歌（从我的来源下载）</button>
        </div>
        <span class="badge2 ok">不联网也能用</span>
        <span class="badge2">加入新歌需要先在“自动获取”里添加音乐来源</span>
        <button class="mp-btn ghost" onclick={aiEnhance} disabled={aiBusy}>{aiBusy ? 'AI 点评中…' : doc.ai ? 'AI 点评（重新）' : 'AI 点评'}</button>
      </div>

      <div>
        {#each doc.tracks as item, i (item.id)}
          {@const t = lib.byId(item.id)}
          <div
            class="drow"
            class:on={player.current?.id === item.id}
            style="animation-delay:{i * 40}ms"
            role="button"
            tabindex="0"
            onclick={() => playFrom(item.id)}
            onkeydown={e => e.key === 'Enter' && playFrom(item.id)}
          >
            <Cover cover={t?.cover} name={t?.album ?? ''} hue={t?.hue} />
            <span>
              <b>{t?.title ?? `（已不在库里 #${item.id}）`}</b>
              <small>{t?.artist ?? ''}{t?.format ? ` · ${t.format}` : ''}</small>
            </span>
            <span class="why">{item.reason}</span>
            <span class="badge2 {SOURCE[item.kind]?.cls ?? ''}">{SOURCE[item.kind]?.label ?? item.kind}</span>
            <span class="ra">
              <button class:fav={liked.includes(item.id)} onclick={e => { e.stopPropagation(); like(item); }}>喜欢</button>
              <span class="dwrap">
                <button onclick={e => { e.stopPropagation(); menuFor = menuFor === item.id ? null : item.id; }}>不感兴趣</button>
                {#if menuFor === item.id}
                  <div class="dpop" role="menu">
                    <button onclick={e => { e.stopPropagation(); feedback(item, 'not_song'); }}>不喜欢这首</button>
                    <button onclick={e => { e.stopPropagation(); feedback(item, 'not_artist'); }}>不喜欢这个艺人</button>
                    <button onclick={e => { e.stopPropagation(); feedback(item, 'not_today'); }}>今天不想听这种</button>
                  </div>
                {/if}
              </span>
            </span>
          </div>
        {/each}
      </div>
    {:else if !smallLib}
      <div class="empty">今天的推荐都被你划掉了，点“换一批”。</div>
    {/if}

    <div class="frow" style="margin-top:18px">
      <button class="mp-btn ghost" onclick={() => (showHistory = !showHistory)}>回看最近 7 天</button>
      {#if pastDay}<button class="mp-btn ghost" onclick={() => load()}>回到今天</button>{/if}
    </div>
    {#if showHistory}
      <div class="hist">
        {#if history.length}
          {#each history as h (h.day)}
            <button class="mp-btn ghost" onclick={() => load(h.day)}>{dayLabel(h.day)} · {h.title} · {h.count} 首</button>
          {/each}
        {:else}
          <div class="note">还没有历史推荐</div>
        {/if}
      </div>
    {/if}

    <div class="note" style="margin-top:14px">每天第一次打开时生成，只用本地库里的歌和你的播放记录，不联网。“不感兴趣”和跳过的记录会影响之后的推荐。</div>
  {/if}
</section>

<style>
  .dwrap {
    position: relative;
    display: inline-block;
  }
  .dpop {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    z-index: 5;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 160px;
    padding: 6px;
    border-radius: 10px;
    background: #1b1b24;
    border: 1px solid var(--edge);
    box-shadow: 0 10px 30px #0008;
  }
  .dpop button {
    text-align: left;
    background: none;
    border: 0;
    padding: 6px 8px;
    border-radius: 7px;
    color: inherit;
    font-size: 12px;
  }
  .dpop button:hover {
    background: rgba(255, 255, 255, 0.1);
  }
  .hist {
    display: flex;
    flex-direction: column;
    gap: 6px;
    align-items: flex-start;
    margin-top: 10px;
  }
</style>
