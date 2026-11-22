// 列表多选：Ctrl 点选、Shift 连选、Ctrl+A 全选、Esc 取消；“多选”按钮进入多选状态（只用鼠标时）
// 选中状态整体存一个 Set（$state.raw，每次换新 Set），几千项全选也只触发一次更新
import { ui } from './ui.svelte';
import { items } from './items.svelte';
import { typing } from './util';

export class Selection {
  set = $state.raw<Set<number>>(new Set());
  /** 点了“多选”按钮：即使一项没选也保持多选状态 */
  mode = $state(false);
  /** Shift 连选的起点 */
  anchor: number | null = null;

  get size() {
    return this.set.size;
  }
  /** 是否处于多选状态（单击行 = 切换选中） */
  get on() {
    return this.mode || this.set.size > 0;
  }
  has(id: number) {
    return this.set.has(id);
  }
  /** 当前选中的 id，按 order 的顺序 */
  ids(order: number[]) {
    return order.filter(id => this.set.has(id));
  }
  toggle(id: number) {
    const s = new Set(this.set);
    if (!s.delete(id)) s.add(id);
    this.set = s;
    this.anchor = id;
  }
  /** 从起点连选到 id；keep 为 true 时保留原来的选择（Ctrl+Shift） */
  range(order: number[], id: number, keep = false) {
    const a = this.anchor == null ? -1 : order.indexOf(this.anchor);
    const b = order.indexOf(id);
    if (a < 0 || b < 0) return this.toggle(id);
    const s = keep ? new Set(this.set) : new Set<number>();
    for (let i = Math.min(a, b); i <= Math.max(a, b); i++) s.add(order[i]);
    this.set = s;
  }
  selectAll(order: number[]) {
    this.set = new Set(order);
    this.mode = true;
  }
  clear() {
    if (this.set.size) this.set = new Set();
    this.mode = false;
    this.anchor = null;
  }
  /** 去掉已经不在列表里的 id（列表刷新、删除之后） */
  prune(order: number[]) {
    if (!this.set.size) return;
    const valid = new Set(order);
    let gone = false;
    for (const id of this.set) if (!valid.has(id)) { gone = true; break; }
    if (gone) this.set = new Set([...this.set].filter(id => valid.has(id)));
  }
  /**
   * 处理一次单击：Ctrl / Shift 或多选状态下返回 true（已切换选中），
   * 否则返回 false，由调用方执行原来的单击动作（播放 / 打开）
   */
  click(e: MouseEvent, id: number, order: number[]) {
    const ctrl = e.ctrlKey || e.metaKey;
    if (e.shiftKey) this.range(order, id, ctrl);
    else if (ctrl || this.on) this.toggle(id);
    else return false;
    return true;
  }
  /** 全局按键：Ctrl+A 全选、Esc 取消。在 <svelte:window onkeydown> 里调用 */
  keys(e: KeyboardEvent, order: () => number[]) {
    if (ui.modal || items.dialog || ui.videoId != null || ui.bookId != null || ui.comicId != null || typing(e)) return;
    const k = e.key.toLowerCase();
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && k === 'a') {
      e.preventDefault();
      this.selectAll(order());
    } else if (k === 'escape' && this.on && !items.menu) {
      this.clear();
    }
  }
}

/** Shift / Ctrl 点击时不要顺带选中文字 */
export function noTextSelect(e: MouseEvent) {
  if (e.shiftKey || e.ctrlKey || e.metaKey) e.preventDefault();
}

/** 拖动排序：把 ids（按 all 里的先后）整体移到第 gap 项之前（gap 为插入位置 0..n） */
export function moveIds(all: number[], ids: number[], gap: number) {
  const moving = new Set(ids);
  const before = all.slice(0, gap).filter(id => !moving.has(id)).length;
  const rest = all.filter(id => !moving.has(id));
  rest.splice(before, 0, ...all.filter(id => moving.has(id)));
  return rest;
}

/** 同一页有两组可选的东西时（电视剧 / 独立视频），在 mine 里操作前清掉 other 的选择，多选状态转过来 */
export function claim(mine: Selection, other: Selection) {
  if (!other.on) return;
  other.clear();
  mine.mode = true;
}
