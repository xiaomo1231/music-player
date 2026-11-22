# 参与开发

欢迎一起开发极光播放器。先读这三份：

- [开发说明](docs/开发说明.md)：要装的工具、常用命令、播放引擎、测试数据目录
- [开发约定](docs/开发约定.md)：目录结构、哪些文件谁能改、前后端规则、测试数据隔离（必须遵守）
- [发布流程](docs/发布流程.md)：版本号规范、怎么发新版本、自动更新原理

## 第一次跑起来

```powershell
git clone https://github.com/xiaomo1231/music-player.git
cd music-player
pnpm install
$env:MP_DATA_DIR = "$PWD\..\testdata"   # 测试数据和正式数据分开
pnpm tauri dev
```

## 分支和 PR

- `main` 永远是能打包的状态，不直接往 `main` 推代码。
- 每件事从最新的 `main` 开一个分支，名字用 `feat/…`（新功能）、`fix/…`（修 bug）、`perf/…`（性能）、`docs/…`（文档）。
- 一个 PR 只做一件事，标题用中文写清楚改了什么，例如“歌词：在线获取和翻译显示”。
- 提交前在本机跑一遍（和 CI 一样）：

  ```powershell
  pnpm check
  pnpm build
  cd src-tauri
  cargo fmt --check
  cargo test --lib
  cargo clippy --all-targets -- -D warnings
  ```

- CI 全绿才能合并。合并用 Squash，保持 `main` 的历史一条线。

## 不要做的事

- 不要改版本号（发布时统一改，见发布流程）。
- 不要提交签名私钥、API 密钥、`.env`、测试数据、打包产物（`release-out/`、`src-tauri/target/`）。
- 不要用正式数据目录做测试，测试实例用完就关。
- `lib.rs`、`Cargo.toml`、`db.rs`、`App.svelte`、`api.ts` 是共用文件，改之前先在 PR 里说明，避免和别人冲突。

## 发布新版本

只有仓库维护者（持有签名私钥的人）能发布。协作者合并完功能后告诉维护者即可。
