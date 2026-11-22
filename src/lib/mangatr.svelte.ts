// 漫画 AI 翻译嵌字：当前书的翻译数据、选中句子、编辑后立即刷新画面，以及导出时的页面渲染
import { invoke, listen, inTauri, comicPage } from './api';
import { toast } from './ui.svelte';
import { fitText, textColor, measureText, FONT, LINE_HEIGHT, COLUMN_WIDTH } from '../components/mangatr/layout';

export type BlockKind = 'bubble' | 'caption' | 'sfx' | 'art';

export interface Block {
  id: string;
  /** [x, y, w, h]，0–1，相对整页 */
  bbox: [number, number, number, number];
  vertical: boolean;
  order: number;
  src: string;
  tr: string;
  alt?: string[] | null;
  kind: BlockKind;
  skipped: boolean;
  bg?: string | null;
}
export interface PageTr {
  blocks: Block[];
}
export interface Term {
  src: string;
  tr: string;
}
/** pages 的键是页号字符串（从 0 开始，与 comicPage 一致） */
export interface BookTr {
  pages: Record<string, PageTr>;
  glossary: Term[];
}
export interface Progress {
  comic_id: number;
  page: number;
  from: number;
  to: number;
  done: number;
  total: number;
  stage: string;
  message: string;
}
export interface Estimate {
  pages: number;
  per_page: number;
  total: number;
  note: string;
}

const SAVE_DELAY = 350;
const TERMINAL = ['done', 'error', 'cancelled'];

const emptyBook = (): BookTr => ({ pages: {}, glossary: [] });

class MangaTr {
  comicId = $state<number | null>(null);
  book = $state<BookTr>(emptyBook());
  /** 每次完整载入一本书 +1，面板据此重新同步术语表编辑区 */
  loadSeq = $state(0);
  selected = $state<string | null>(null);
  /** 后台整话翻译进行中 */
  running = $state(false);
  /** 单页识别 / 重译进行中 */
  busy = $state(false);
  progress = $state<Progress | null>(null);

  private loadTok = 0;
  /** 尚未写入磁盘的译文修改（块 id → 书、页号、译文），刷新时不能被旧数据盖掉 */
  private pending = new Map<string, { comicId: number; page: number; tr: string }>();
  private timers = new Map<string, ReturnType<typeof setTimeout>>();

  constructor() {
    void listen<Progress>('mangatr://progress', (p) => this.onProgress(p));
  }

  blocksOf(page: number): Block[] {
    return this.book.pages[String(page)]?.blocks ?? [];
  }

  hasTr(page: number): boolean {
    return this.blocksOf(page).some((b) => !b.skipped && b.tr.trim() !== '');
  }

  private blockRef(page: number, id: string): Block | undefined {
    return this.book.pages[String(page)]?.blocks.find((b) => b.id === id);
  }

  private withPending(b: BookTr): BookTr {
    for (const [id, p] of this.pending) {
      if (p.comicId !== this.comicId) continue;
      const blk = b.pages[String(p.page)]?.blocks.find((x) => x.id === id);
      if (blk) blk.tr = p.tr;
    }
    return b;
  }

  /** 载入一本书；已经载入同一本时不重复读取，force 时强制重读 */
  async load(comicId: number, force = false) {
    if (!inTauri) return;
    if (!force && comicId === this.comicId && this.loadSeq > 0) return;
    if (comicId !== this.comicId) {
      this.comicId = comicId;
      this.book = emptyBook();
      this.selected = null;
    }
    const tok = ++this.loadTok;
    try {
      const b = await invoke<BookTr>('mangatr_get', { comicId });
      if (tok !== this.loadTok) return;
      this.book = this.withPending(b);
      this.loadSeq++;
    } catch (e) {
      toast(String(e));
    }
  }

  async refresh() {
    if (this.comicId == null || !inTauri) return;
    const b = await invoke<BookTr>('mangatr_get', { comicId: this.comicId });
    this.book = this.withPending(b);
  }

  /** 改了一句译文：画面立刻更新，磁盘在停顿后保存 */
  editTr(page: number, id: string, tr: string) {
    const blk = this.blockRef(page, id);
    if (blk) blk.tr = tr;
    if (this.comicId == null) return;
    this.pending.set(id, { comicId: this.comicId, page, tr });
    clearTimeout(this.timers.get(id));
    this.timers.set(
      id,
      setTimeout(() => void this.flush(id), SAVE_DELAY),
    );
  }

  /** 立即保存一句译文（选备选、点按钮时用） */
  async commitTr(page: number, id: string, tr: string) {
    this.editTr(page, id, tr);
    await this.flush(id);
  }

  private async flush(id: string) {
    const p = this.pending.get(id);
    clearTimeout(this.timers.get(id));
    this.timers.delete(id);
    if (!p) return;
    this.pending.delete(id);
    try {
      await invoke('mangatr_update_block', { comicId: p.comicId, page: p.page, blockId: id, tr: p.tr });
    } catch (e) {
      toast('保存失败：' + String(e));
    }
  }

  /** 带前后文重新翻译一句 */
  async retranslate(page: number, id: string) {
    if (this.comicId == null) return;
    clearTimeout(this.timers.get(id));
    this.timers.delete(id);
    this.pending.delete(id);
    this.busy = true;
    try {
      const blk = await invoke<Block>('mangatr_retranslate_block', { comicId: this.comicId, page, blockId: id });
      const ref = this.blockRef(page, id);
      if (ref) Object.assign(ref, blk);
    } finally {
      this.busy = false;
    }
  }

  /** 识别 + 翻译一页 */
  async translatePage(page: number) {
    if (this.comicId == null) return;
    this.busy = true;
    try {
      const pg = await invoke<PageTr>('mangatr_translate_page', { comicId: this.comicId, page });
      this.book.pages[String(page)] = pg;
    } finally {
      this.busy = false;
    }
  }

  async estimate(from: number, to: number): Promise<Estimate> {
    return invoke<Estimate>('mangatr_estimate', { comicId: this.comicId, from, to });
  }

  async translateRange(from: number, to: number) {
    if (this.comicId == null) return;
    this.running = true;
    try {
      await invoke('mangatr_translate_range', { comicId: this.comicId, from, to });
    } catch (e) {
      this.running = false;
      throw e;
    }
  }

  async cancel() {
    if (this.comicId != null) await invoke('mangatr_cancel', { comicId: this.comicId });
  }

  /** 改术语表：返回替换了多少处已有译文（applyAll 时） */
  async setGlossary(glossary: Term[], applyAll: boolean): Promise<number> {
    if (this.comicId == null) return 0;
    const clean = glossary.map((t) => ({ src: t.src.trim(), tr: t.tr.trim() })).filter((t) => t.src !== '');
    const n = await invoke<number>('mangatr_glossary_set', { comicId: this.comicId, glossary: clean, applyAll });
    this.book.glossary = clean;
    if (applyAll) await this.refresh();
    return n;
  }

  /** 导出：有译文的页用渲染好的图，其余页拷原图。返回新文件路径。 */
  async exportCbz(pageCount: number): Promise<string> {
    if (this.comicId == null) throw new Error('还没有载入漫画');
    const comicId = this.comicId;
    const pngs: [number, string][] = [];
    for (let p = 0; p < pageCount; p++) {
      if (this.hasTr(p)) pngs.push([p, await renderPage(comicId, p)]);
    }
    return invoke<string>('mangatr_export', { comicId, pagesPng: pngs });
  }

  private onProgress(p: Progress) {
    if (p.comic_id !== this.comicId) return;
    this.progress = p;
    if (p.stage === 'saved') void this.refresh();
    if (TERMINAL.includes(p.stage)) {
      this.running = false;
      void this.refresh();
      toast(p.stage === 'done' ? '整话翻译完成，可以逐句修改' : p.message);
    }
  }
}

export const mangatr = new MangaTr();

/** 把一页的译文画到原图上，返回 base64 PNG（不含前缀）。给导出用。 */
export async function renderPage(comicId: number, page: number): Promise<string> {
  const url = comicPage(comicId, page);
  if (!url) throw new Error('不在桌面程序中运行');
  const img = new Image();
  img.crossOrigin = 'anonymous';
  img.src = url;
  await img.decode();
  const W = img.naturalWidth;
  const H = img.naturalHeight;
  const canvas = document.createElement('canvas');
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('无法创建画布');
  ctx.drawImage(img, 0, 0, W, H);
  let blocks: Block[];
  if (mangatr.comicId === comicId) {
    blocks = mangatr.blocksOf(page);
  } else {
    const b = await invoke<BookTr>('mangatr_get', { comicId });
    blocks = b.pages[String(page)]?.blocks ?? [];
  }
  for (const b of blocks) {
    if (!b.skipped && b.tr.trim()) drawBubble(ctx, b, W, H);
  }
  const data = canvas.toDataURL('image/png');
  return data.slice(data.indexOf(',') + 1);
}

function drawBubble(ctx: CanvasRenderingContext2D, b: Block, W: number, H: number) {
  const x = b.bbox[0] * W;
  const y = b.bbox[1] * H;
  const w = b.bbox[2] * W;
  const h = b.bbox[3] * H;
  const pad = Math.max(2, Math.round(Math.min(w, h) * 0.06));
  // 白底（取样底色）盖住原文
  ctx.fillStyle = b.bg || '#ffffff';
  ctx.fillRect(x - pad, y - pad, w + 2 * pad, h + 2 * pad);

  const fit = fitText(b.tr, w, h, b.vertical, measureText);
  const size = fit.size;
  ctx.fillStyle = textColor(b.bg);
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.font = `${size}px ${FONT}`;
  if (b.vertical) {
    // 竖排：列从右到左，每列从上到下
    const colW = size * COLUMN_WIDTH;
    const total = fit.lines.length * colW;
    const right = x + (w + total) / 2;
    fit.lines.forEach((col, i) => {
      const cx = right - colW * (i + 0.5);
      const chars = Array.from(col);
      const top = y + (h - chars.length * size) / 2;
      chars.forEach((c, j) => ctx.fillText(c, cx, top + j * size + size / 2));
    });
  } else {
    const lh = size * LINE_HEIGHT;
    const top = y + (h - fit.lines.length * lh) / 2;
    fit.lines.forEach((l, i) => ctx.fillText(l, x + w / 2, top + i * lh + lh / 2));
  }
}
