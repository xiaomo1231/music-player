<script lang="ts">
  // 全局热键列表：可开关、点击“录制”后按下新组合、占用的标红、恢复默认
  import { onMount, onDestroy } from 'svelte';
  import { inTauri } from '../../lib/api';
  import { toast } from '../../lib/ui.svelte';
  import { accelFromEvent, HOTKEY_LABEL, loadHotkeys, resetHotkeys, saveHotkeys, suspendHotkeys, type HotkeyStatus } from '../../lib/shell.svelte';

  let rows = $state<HotkeyStatus[]>([]);
  let recording = $state<string | null>(null);
  let busy = $state(false);

  const ARROW: Record<string, string> = { ArrowLeft: '←', ArrowRight: '→', ArrowUp: '↑', ArrowDown: '↓' };
  /** 把 KeyP / Digit1 / ArrowLeft 这样的写法显示得更好看 */
  const pretty = (s: string) =>
    s
      .split('+')
      .map(p => ARROW[p] ?? p.replace(/^Key/, '').replace(/^Digit/, ''))
      .join(' + ');

  async function load() {
    try {
      rows = await loadHotkeys();
    } catch (e) {
      toast('读取快捷键失败：' + String(e));
    }
  }

  onMount(() => {
    if (inTauri) load();
  });

  async function save(next: HotkeyStatus[]) {
    busy = true;
    try {
      rows = await saveHotkeys(next.map(({ action, accel, enabled }) => ({ action, accel, enabled })));
      // 保存会重新注册全部热键；此时若又在录制，重新暂停
      if (recording != null) await suspendHotkeys(true);
    } catch (e) {
      toast('保存快捷键失败：' + String(e));
    } finally {
      busy = false;
    }
  }

  async function reset() {
    busy = true;
    try {
      rows = await resetHotkeys();
      toast('已恢复默认快捷键');
    } catch (e) {
      toast('恢复失败：' + String(e));
    } finally {
      busy = false;
    }
  }

  function setEnabled(i: number, on: boolean) {
    recording = null;
    save(rows.map((r, k) => (k === i ? { ...r, enabled: on } : r)));
  }

  // 录制期间暂停全部全局热键：否则按下的组合若已被注册，会被系统直接拦下（界面收不到按键、反而触发了动作）
  function startRecording(action: string) {
    commitPending();
    if (recording === action) return stopRecording();
    const was = recording;
    recording = action;
    if (was == null && inTauri) suspendHotkeys(true).catch(() => {});
  }

  /** 取消录制并恢复热键 */
  function stopRecording() {
    if (recording == null) return;
    recording = null;
    if (inTauri) suspendHotkeys(false).then(r => (rows = r)).catch(() => {});
  }

  /** 已录到、等松开按键再保存的组合（按住时就注册，自动重复的按键会被系统当成热键触发） */
  let pending: { next: HotkeyStatus[]; code: string; timer: ReturnType<typeof setTimeout> } | null = null;

  function commitPending() {
    if (!pending) return;
    const { next, timer } = pending;
    clearTimeout(timer);
    pending = null;
    save(next);
  }

  // 录制时在捕获阶段拦截按键，避免同时触发应用内快捷键
  function onKey(e: KeyboardEvent) {
    if (recording == null) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    if (e.key === 'Escape') return stopRecording();
    const accel = accelFromEvent(e);
    if (!accel) return; // 只按了修饰键，继续等待
    const action = recording;
    recording = null;
    // 先把新组合显示出来，松开按键后再保存注册
    const next = rows.map(r => (r.action === action ? { ...r, accel, enabled: true, error: undefined } : r));
    rows = next;
    pending = { next, code: e.code, timer: setTimeout(commitPending, 1500) };
  }

  function onKeyUp(e: KeyboardEvent) {
    if (!pending) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    if (e.code === pending.code || !(e.ctrlKey || e.altKey || e.metaKey)) commitPending();
  }

  onDestroy(() => {
    if (pending) commitPending();
    else if (recording != null && inTauri) suspendHotkeys(false).catch(() => {});
  });
</script>

<svelte:window onkeydowncapture={onKey} onkeyupcapture={onKeyUp} />

<div class="hk">
  {#each rows as r, i (r.action)}
    <div class="mp-opt hk-row" class:bad={!!r.error}>
      <div>
        {HOTKEY_LABEL[r.action] ?? r.action}
        {#if r.error}<small class="err">{r.error}</small>{/if}
      </div>
      <kbd class:dim={!r.enabled}>{pretty(r.accel)}</kbd>
      <button class="mp-btn ghost" disabled={busy} onclick={() => startRecording(r.action)}>
        {recording === r.action ? '按下新组合…' : '修改'}
      </button>
      <input type="checkbox" class="mp-sw" checked={r.enabled} disabled={busy} onchange={e => setEnabled(i, e.currentTarget.checked)} aria-label="启用" />
    </div>
  {/each}
  {#if !rows.length}
    <p class="note">{inTauri ? '正在读取…' : '仅在桌面程序中可用'}</p>
  {/if}
</div>
<div class="rowbtns">
  <button class="mp-btn ghost" disabled={busy || !rows.length} onclick={() => { recording = null; reset(); }}>恢复默认</button>
  {#if recording}<small class="hint">按 Esc 取消；按下 Ctrl / Alt / Win 加按键的组合</small>{/if}
</div>

<style>
  .hk-row { align-items: center; }
  .hk-row > div { flex: 1; min-width: 0; }
  kbd { font: inherit; font-size: 12.5px; padding: 3px 8px; border-radius: 6px; border: 1px solid var(--mp-line); background: var(--mp-panel2); white-space: nowrap; }
  kbd.dim { opacity: 0.45; }
  .hk-row.bad kbd { border-color: #ff6b6b; color: #ff8a8a; }
  .err { display: block; color: #ff8a8a; font-size: 12px; margin-top: 2px; line-height: 1.5; }
  .hint { color: var(--mp-sub); font-size: 12px; }
  .note { color: var(--mp-sub); font-size: 13px; }
</style>
