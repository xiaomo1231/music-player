# 窗口外壳（shell）集成说明

负责文件：`src-tauri/src/shell.rs`、`src/lib/shell.svelte.ts`、`src/views/Settings.svelte`、`src/components/settings/*`。

包含：托盘、全局热键、单实例、开机启动、防休眠、壁纸模式、资源管理器右键菜单、桌面歌词、缓存清理、设置页。

## 1. lib.rs 要做的改动

`shell::setup` 已经在 `setup_modules` 里调用，`shell::init` 已在 `init_modules` 里调用，这两处不用再改。

还需要加三个插件。**单实例插件必须是第一个 `.plugin`**：

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_single_instance::init(shell::on_second_instance))
    .plugin(tauri_plugin_global_shortcut::Builder::new().build())
    .plugin(tauri_plugin_autostart::Builder::new().args(["--minimized"]).build())
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_opener::init())
    // ……其余不变
```

说明：
- 开机启动的参数固定带 `--minimized`，“启动后最小化到托盘”由设置 `autostart_minimized` 决定（`shell::setup` 里读取），所以开关不需要为此重新注册。
- 全局热键要在 `shell::setup` 之前完成插件初始化，否则 `app.global_shortcut()` 会 panic。上面的写法已满足。

## 2. 要在 `generate_handler!` 里注册的命令

```rust
shell::shell_hotkeys,
shell::shell_set_hotkeys,
shell::shell_hotkeys_reset,
shell::shell_hotkeys_suspend,
shell::shell_show_main,
shell::shell_tray_tooltip,
shell::shell_startup_files,
shell::shell_autostart_get,
shell::shell_autostart_set,
shell::shell_keep_awake,
shell::shell_wallpaper,
shell::shell_explorer_menu,
shell::shell_desktop_lyrics,
shell::shell_cache_size,
shell::shell_cache_clear,
```

| 命令 | 参数 | 返回 |
| --- | --- | --- |
| `shell_hotkeys` | 无 | `[{action, accel, enabled, error?}]`，最近一次注册的结果 |
| `shell_set_hotkeys` | `list: [{action, accel, enabled}]` | 同上（保存到设置 `hotkeys` 并重新注册） |
| `shell_hotkeys_reset` | 无 | 同上（恢复默认） |
| `shell_hotkeys_suspend` | `on` | 同上。设置页录制新组合期间暂停全部热键（`on: true`），取消录制时按已保存的配置恢复（`on: false`） |
| `shell_show_main` | 无 | 显示主窗口并拉到前台 |
| `shell_tray_tooltip` | `text` | 更新托盘提示 |
| `shell_startup_files` | 无 | `{paths, addOnly} \| null`，启动时带来的文件，取走后清空 |
| `shell_autostart_get` | 无 | `{enabled, minimized}` |
| `shell_autostart_set` | `enabled`, `minimized` | 无 |
| `shell_keep_awake` | `on` | 无 |
| `shell_wallpaper` | `on` | 失败时返回中文错误信息 |
| `shell_explorer_menu` | `on` | 失败时返回中文错误信息（只写 HKCU） |
| `shell_desktop_lyrics` | `on` | 创建 / 关闭 `lyrics` 窗口 |
| `shell_cache_size` | 无 | `{thumbs, covers}`（字节） |
| `shell_cache_clear` | `covers: bool` | 释放的字节数（只清 thumbs，`covers` 为真时连 covers 一起清；不碰数据库） |

## 3. capabilities 要改的地方

`src-tauri/capabilities/default.json`：把桌面歌词窗口的 label 加进 `windows`，这样歌词窗口才有事件收发权限（`core:default` 里已包含 event 的 listen / emit）：

```json
"windows": ["main", "mini", "lyrics"],
```

其他权限不需要新增：外壳的托盘、热键、开机启动、单实例都在 Rust 侧完成，前端只调用自定义命令（自定义命令不需要 capability）。

## 4. main.ts 要做的改动

桌面歌词窗口加载的是 `index.html#lyrics`，它只需要渲染 `DesktopLyrics`，不要初始化媒体库和外壳。改成：

```ts
import './styles/core.css';
import './styles/aurora.css';
import './styles/app.css';
import { mount } from 'svelte';
import App from './App.svelte';
import DesktopLyrics from './components/settings/DesktopLyrics.svelte';
import { initPaths } from './lib/api';
import { lib } from './lib/library.svelte';
import { initShell, isLyricsWindow } from './lib/shell.svelte';

if (isLyricsWindow()) {
  mount(DesktopLyrics, { target: document.body });
} else {
  await initPaths().catch(() => {});
  lib.refresh().catch(e => console.error(e));
  mount(App, { target: document.body });
  initShell(); // 只调用一次：托盘 / 热键命令、防休眠、桌面歌词推送、启动文件
  // ……开发模式的 __mp 调试代码保持不变
}
```

注意：`lib.refresh()` 和 `initPaths()` 只在主窗口执行。`initShell()` 在 `isLyricsWindow()` 为真时本身就是空操作，但主窗口里要调用一次。

`initShell()` 会做这些事：
- 监听 `shell://cmd`：`toggle` / `next` / `prev` 调用 player；`vol_up` / `vol_down` 调音量；`fav` 收藏当前歌曲；`lyrics` 切换桌面歌词；`show` 显示主窗口（并退出迷你模式）；`mini` 进入迷你播放器。
- 监听 `shell://close-ask`：`{ask, downloads}`，弹出 CloseModal（最小化到托盘 / 退出，可记住选择；有下载在进行时提醒），结果交给 `shell_close_choose`；监听挂上后调用 `shell_close_ready`，之前点关闭直接退出。
- 监听 `shell://open-files`：`{paths, addOnly}`，调用 `add_files` 入库；`addOnly` 为假时播放第一首音乐，否则只提示。
- 启动时取 `shell_startup_files`，并恢复设置 `default_volume`（固定音量）和 `desktop_lyrics`（桌面歌词）。
- 挂上防休眠（视频播放器打开，或沉浸模式且在播放时）、托盘提示（当前歌曲）、桌面歌词推送。

## 5. App.svelte / ui.svelte.ts / api.ts

不需要改。`Settings.svelte` 已经通过 `ui.open('scan' | 'oled' | 'ai' | 'keys')` 和 `ui.setMini()` 与外壳交互。

## 6. 与其他模块的约定

- 媒体库监控开关存在设置 `watch`（默认开）。改动后只保存设置，监控重启由集成负责人处理（可在 `set_setting` 时对 `watch` 调用 `commands::restart_watcher`）。
- 联网补全开关存在设置 `metadata_online`（默认关）。
- 设置键一览：`hotkeys`、`close_action`（"ask" 每次询问 / "tray" 最小化到托盘 / "quit" 直接退出；没有时沿用旧的布尔键 `close_to_tray`）、`autostart_minimized`、`default_volume`（null 表示沿用上次音量）、`desktop_lyrics`、`explorer_menu`、`watch`、`metadata_online`、`fade`（占位）。
- 资源管理器右键菜单：音频和视频扩展名下各加两项，命令为 `"<exe>" "%1"`；“加入媒体库”项为 `"<exe>" --add "%1"`。两者都通过单实例 / 启动参数把文件交给 `shell://open-files`。
- 热键事件名：`shell://cmd` 的命令值有 `toggle`、`next`、`prev`、`vol_up`、`vol_down`、`fav`、`lyrics`、`show`、`mini`。`window`（Ctrl+Alt+P）由 Rust 直接切换主窗口，不发事件。
- 桌面歌词：主窗口 emit `lyrics://line`（当前歌词文本），歌词窗口 emit `lyrics://ask` 请求当前行。

## 7. 热键

默认值（可在设置页修改、开关，结果标红表示被占用或格式不对）：

| 动作 | 默认 |
| --- | --- |
| 播放 / 暂停 / 上一首 / 下一首（多媒体键） | MediaPlayPause / MediaPreviousTrack / MediaNextTrack |
| 上一首 / 下一首 | Ctrl+Alt+ArrowLeft / ArrowRight |
| 音量加 / 减 | Ctrl+Alt+ArrowUp / ArrowDown |
| 显示 / 隐藏主窗口 | Ctrl+Alt+P |
| 收藏当前歌曲 | Ctrl+Alt+D |
| 显示 / 隐藏桌面歌词 | Ctrl+Alt+L |

按键规则：单键必须带 Ctrl / Alt / Win，或是多媒体键、F1–F24；只带 Shift 不允许。录制时用 `KeyboardEvent.code`，所以按键名形如 `KeyP`、`Digit1`、`ArrowLeft`、`Space`、`F5`。

## 8. 自测结果

- `src-tauri`：`cargo check` 通过（本模块无警告）。
- `cargo test --lib shell`：6 个用例全部通过（热键解析与校验、热键默认值与合并、启动参数解析、缓存清理只清目录内容）。
  注意：当前工作树里 `src/fetch.rs` 引用的 `fetch/source.rs`、`fetch/web.rs` 还不存在，`cargo test` 整体编译会失败。这次测试是在临时副本里把 `fetch.rs` 替换成空桩后跑的，没有修改别人的文件。
- 项目根：`pnpm check` 本模块零错误零警告；剩余 7 个错误全部来自 `App.svelte` 引用的 `AiSub.svelte`、`Fetch.svelte`、`Organize.svelte`（别的模块尚未提交）。`pnpm build` 通过。
- 未做的事：没有修改注册表，没有启动程序，没有提交 git。

## 9. 已知问题

- 壁纸模式是非官方做法：依赖 Progman / WorkerW 的实现细节，个别 Windows 版本可能失败（会返回中文错误）。开启时会先进入沉浸模式，但窗口不再全屏（改为铺满主显示器）。
- 桌面歌词窗口的地址是 `index.html#lyrics`。只做了编译和类型检查，没有实际启动，透明效果和窗口位置还需要在桌面上验证。
- 鼠标穿透后，桌面歌词窗口不能被拖动，位置固定在主显示器底部居中。
- 托盘“显示主窗口”会先在 Rust 里显示窗口，再发 `show` 事件让前端退出迷你模式。
- 防休眠用单独线程每 30 秒续一次 `SetThreadExecutionState`，关闭时恢复系统默认。
- 资源管理器菜单只写 `HKCU\Software\Classes\SystemFileAssociations\{audio,video}\shell`，没有实际写入过。
- 淡入淡出开关只是占位，只存设置，没有实际效果。
- 缓存清理：默认只清 `thumbs`，勾选后才清 `covers`。
