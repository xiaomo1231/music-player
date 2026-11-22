import './styles/core.css';
import './styles/aurora.css';
import './styles/app.css';
import { mount } from 'svelte';
import App from './App.svelte';
import { initPaths } from './lib/api';
import { lib } from './lib/library.svelte';

await initPaths().catch(() => {});
lib.refresh().catch(e => console.error(e));
mount(App, { target: document.body });

if (import.meta.env.DEV) {
  // 调试与自动化测试用
  const [{ player }, { ui }] = await Promise.all([import('./lib/player.svelte'), import('./lib/ui.svelte')]);
  (window as unknown as Record<string, unknown>).__mp = { player, lib, ui };
}
