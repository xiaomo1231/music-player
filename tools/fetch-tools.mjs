// 下载随安装包分发的外部程序（yt-dlp、ffmpeg / ffprobe）到 src-tauri/bin/，打包时整个文件夹装到程序目录的 bin\。
//
// 用法（仓库根目录）：
//   node tools/fetch-tools.mjs          下载并校验（已是清单里的版本就跳过）
//   node tools/fetch-tools.mjs --check  只检查 src-tauri/bin/ 是否齐全、版本是否和清单一致，不下载
//
// 版本、下载地址和 SHA-256 写在 tools/bundled-tools.json。下载的安装包缓存在 src-tauri/target/tools-cache/。
// bin/versions.json 记录装进去的版本（软件据此判断自带的 yt-dlp 和用户后来更新的哪个新）。
// 每个程序的许可文本放到 bin/licenses/<名字>/，并生成 bin/licenses/THIRD-PARTY-NOTICES.txt（随安装包分发，GPL 要求）。
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const manifest = JSON.parse(fs.readFileSync(path.join(root, 'tools/bundled-tools.json'), 'utf8'));
const binDir = path.join(root, 'src-tauri/bin');
const licDir = path.join(binDir, 'licenses');
const cacheDir = path.join(root, 'src-tauri/target/tools-cache');
const stampFile = path.join(binDir, 'versions.json');
const checkOnly = process.argv.includes('--check');

const sha256 = buf => createHash('sha256').update(buf).digest('hex');

async function download(url) {
  for (let i = 1; ; i++) {
    try {
      const r = await fetch(url, { redirect: 'follow' });
      if (!r.ok) throw new Error(`HTTP ${r.status}`);
      return Buffer.from(await r.arrayBuffer());
    } catch (e) {
      if (i >= 4) throw new Error(`下载失败 ${url}：${e.message}`);
      console.log(`  下载出错（${e.message}），${i * 2} 秒后重试…`);
      await new Promise(res => setTimeout(res, i * 2000));
    }
  }
}

/** 下载（或从缓存取）并校验 SHA-256 */
async function fetchVerified(tool) {
  fs.mkdirSync(cacheDir, { recursive: true });
  const cached = path.join(cacheDir, `${tool.sha256}-${path.basename(new URL(tool.url).pathname)}`);
  if (fs.existsSync(cached) && sha256(fs.readFileSync(cached)) === tool.sha256) return cached;
  console.log(`  下载 ${tool.url}`);
  const buf = await download(tool.url);
  const got = sha256(buf);
  if (got !== tool.sha256) throw new Error(`${tool.name} 校验不通过：期望 ${tool.sha256}，实际 ${got}`);
  fs.writeFileSync(cached, buf);
  return cached;
}

function unzip(zip, dest) {
  fs.rmSync(dest, { recursive: true, force: true });
  fs.mkdirSync(dest, { recursive: true });
  // Windows 10 起自带的 tar（bsdtar）能解 zip；其他系统用 unzip
  const [cmd, args] = process.platform === 'win32' ? ['tar', ['-xf', zip, '-C', dest]] : ['unzip', ['-q', '-o', zip, '-d', dest]];
  const r = spawnSync(cmd, args, { stdio: 'inherit' });
  if (r.status !== 0) throw new Error(`解压失败：${zip}`);
}

function readStamp() {
  try {
    return JSON.parse(fs.readFileSync(stampFile, 'utf8'));
  } catch {
    return {};
  }
}

function upToDate(tool, stamp) {
  if (stamp[tool.name]?.sha256 !== tool.sha256) return false;
  return Object.values(tool.files).every(f => fs.existsSync(path.join(binDir, f)));
}

function notices() {
  const lines = [
    '极光播放器 第三方组件说明 / Third-party components',
    '',
    '程序目录 bin\\ 里的以下程序是独立的第三方软件，按各自的许可证随安装包分发，播放器通过命令行调用它们。',
    'The programs below are separate third-party works, redistributed unmodified under their own licenses.',
    '',
  ];
  for (const t of manifest.tools) {
    lines.push(`== ${t.name} ${t.version}`);
    lines.push(`文件 / Files: ${Object.values(t.files).join(', ')}`);
    lines.push(`许可证 / License: ${t.license}（全文见 licenses\\${t.name}\\）`);
    lines.push(`下载地址 / Binary: ${t.url}`);
    lines.push(`SHA-256: ${t.sha256}`);
    lines.push(`源码 / Source: ${t.source}`);
    lines.push('');
  }
  lines.push('ffmpeg 按 GPL v3 分发。如需对应的完整源码，可从上面的地址获取；或在本软件发布后三年内通过');
  lines.push('https://github.com/xiaomo1231/music-player/issues 向我们索取，我们会提供同一版本的源码副本。');
  lines.push('ffmpeg is distributed under GPL v3. The complete corresponding source is available at the URLs above;');
  lines.push('for three years after distribution you may also request a copy via https://github.com/xiaomo1231/music-player/issues .');
  lines.push('');
  return lines.join('\r\n');
}

async function main() {
  const stamp = readStamp();
  const missing = manifest.tools.filter(t => !upToDate(t, stamp));
  if (checkOnly) {
    if (missing.length) {
      console.error(`src-tauri/bin/ 缺少或版本不对：${missing.map(t => t.name).join('、')}。先运行 node tools/fetch-tools.mjs`);
      process.exit(1);
    }
    console.log('src-tauri/bin/ 齐全：' + manifest.tools.map(t => `${t.name} ${t.version}`).join('，'));
    return;
  }
  fs.mkdirSync(binDir, { recursive: true });
  for (const t of manifest.tools) {
    const toolLic = path.join(licDir, t.name);
    if (upToDate(t, stamp) && fs.existsSync(toolLic)) {
      console.log(`${t.name} ${t.version}：已是最新`);
      continue;
    }
    console.log(`${t.name} ${t.version}`);
    const file = await fetchVerified(t);
    fs.rmSync(toolLic, { recursive: true, force: true });
    fs.mkdirSync(toolLic, { recursive: true });
    if (t.zipRoot) {
      const tmp = path.join(cacheDir, `${t.name}-unzipped`);
      unzip(file, tmp);
      const base = path.join(tmp, t.zipRoot);
      for (const [from, to] of Object.entries(t.files)) fs.copyFileSync(path.join(base, from), path.join(binDir, to));
      for (const [from, to] of Object.entries(t.licenseFiles ?? {})) fs.copyFileSync(path.join(base, from), path.join(toolLic, to));
      fs.rmSync(tmp, { recursive: true, force: true });
    } else {
      const [to] = Object.values(t.files);
      fs.copyFileSync(file, path.join(binDir, to));
    }
    for (const [to, url] of Object.entries(t.licenses ?? {})) fs.writeFileSync(path.join(toolLic, to), await download(url));
    stamp[t.name] = { version: t.version, sha256: t.sha256 };
    fs.writeFileSync(stampFile, JSON.stringify(stamp, null, 2) + '\n');
  }
  fs.mkdirSync(licDir, { recursive: true });
  fs.writeFileSync(path.join(licDir, 'THIRD-PARTY-NOTICES.txt'), notices());
  console.log(`完成：${binDir}`);
}

main().catch(e => {
  console.error(e.message ?? e);
  process.exit(1);
});
