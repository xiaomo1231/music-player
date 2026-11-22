// 生成应用图标源图（1024×1024，极光渐变圆角方块 + 音符），再用 `pnpm tauri icon` 生成各尺寸
import { deflateSync } from 'node:zlib';
import { writeFileSync } from 'node:fs';
const N = 1024, R = 220;
const px = Buffer.alloc((N * 4 + 1) * N);
const crcT = new Uint32Array(256).map((_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
const crc = b => { let c = ~0; for (const x of b) c = crcT[(c ^ x) & 255] ^ (c >>> 8); return (~c) >>> 0; };
const chunk = (t, d) => { const l = Buffer.alloc(4); l.writeUInt32BE(d.length); const td = Buffer.concat([Buffer.from(t), d]); const c = Buffer.alloc(4); c.writeUInt32BE(crc(td)); return Buffer.concat([l, td, c]); };
const inRound = (x, y) => { const cx = Math.min(Math.max(x, R), N - R), cy = Math.min(Math.max(y, R), N - R); return (x - cx) ** 2 + (y - cy) ** 2 <= R * R; };
const dist = (x, y, a, b) => Math.hypot(x - a, y - b);
for (let y = 0; y < N; y++) {
  px[y * (N * 4 + 1)] = 0;
  for (let x = 0; x < N; x++) {
    const o = y * (N * 4 + 1) + 1 + x * 4;
    if (!inRound(x, y)) { px[o + 3] = 0; continue; }
    const t = (x + y) / (2 * N);
    let r = 90 + 130 * t, g = 170 - 60 * t, b = 255 - 20 * t;
    const glow = Math.max(0, 1 - dist(x, y, 300, 260) / 600);
    r += 60 * glow; g += 60 * glow; b += 20 * glow;
    // 音符：两个圆 + 竖杆 + 横梁
    const note = dist(x, y, 380, 720) < 95 || dist(x, y, 690, 660) < 95 ||
      (x > 445 && x < 485 && y > 300 && y < 720) || (x > 755 && x < 795 && y > 240 && y < 660) ||
      (x > 445 && x < 795 && y > 240 + (795 - x) * 0.17 && y < 330 + (795 - x) * 0.17);
    if (note) { r = 255; g = 255; b = 255; }
    px[o] = Math.min(255, r); px[o + 1] = Math.min(255, g); px[o + 2] = Math.min(255, b); px[o + 3] = 255;
  }
}
const ih = Buffer.alloc(13); ih.writeUInt32BE(N, 0); ih.writeUInt32BE(N, 4); ih[8] = 8; ih[9] = 6;
writeFileSync(process.argv[2] || 'app-icon.png', Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ih), chunk('IDAT', deflateSync(px)), chunk('IEND', Buffer.alloc(0))]));
