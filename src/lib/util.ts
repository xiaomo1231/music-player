export const clamp = (v: number, a = 0, b = 1) => Math.max(a, Math.min(b, v));
export const fmtT = (s: number) => {
  s = Math.max(0, Math.floor(s || 0));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
};
export const fmtSize = (b: number) => {
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return `${b.toFixed(i ? 1 : 0)} ${u[i]}`;
};
export const fmtDate = (ts?: number | null) => {
  if (!ts) return '';
  const d = new Date(ts * 1000);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
};
/** 没有封面时，按名字生成稳定的渐变占位 */
export const hueOf = (s: string) => { let h = 0; for (const c of s) h = (h * 31 + c.charCodeAt(0)) % 360; return h; };
export const coverCSS = (h: number, alt = false) =>
  `radial-gradient(circle at ${alt ? 70 : 28}% 26%,hsl(${h} 85% 68%/.95),transparent 55%),radial-gradient(circle at ${alt ? 22 : 76}% 82%,hsl(${(h + 70) % 360} 80% 52%/.9),transparent 58%),linear-gradient(140deg,hsl(${(h + 300) % 360} 55% 22%),hsl(${h} 60% 34%))`;
export const shuffleArr = <T>(a: T[]) => { a = [...a]; for (let i = a.length - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [a[i], a[j]] = [a[j], a[i]]; } return a; };
/** 输入框里打字时，单键快捷键不生效 */
export const typing = (e: KeyboardEvent) => {
  const t = e.target as HTMLElement | null;
  return !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT' || t.isContentEditable);
};
