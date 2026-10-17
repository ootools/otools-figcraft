# FigCraft · OTools 插件

[FigCraft](https://github.com/storytold/figcraft) 是 storytold 的 ArtCraft「Crafting Apps」家族之一：
纯 Rust 重写的矢量/UI 设计工具，编译为 WebAssembly + egui，在浏览器里运行。
本仓库把它移植成 OTools 插件：UI 仍是上游 `figcraft-ui-egui`，只替换了「文件从哪来、存到哪去」。

> 同家族移植模式见 `../otools-cadcraft`（CAD 制图）。本插件沿用同一套结构。

## 目录结构

```
otools-figcraft/
├── plugin.json            # 插件清单（宿主只认这个 + logo + dist）
├── logo.svg               # 插件图标
├── upstream.pin           # 上游 commit SHA（按需浅拉，不入库）
├── craft-fonts.pin        # 中文字体仓库 commit SHA（按需浅拉，不入库）
├── upstream/              # 上游源码（git 浅拉，.gitignore 忽略，**不入库**）
├── shell/                 # 本插件自己写的 Rust 外壳 + HTML 桥
│   ├── Cargo.toml         # 独立 workspace，path 依赖上游 crate
│   ├── Trunk.toml         # trunk 构建配置（产物 dist/）
│   ├── index.html         # window.figcraftOtools 宿主桥（唯一需要写的 JS）
│   ├── src/
│   │   ├── main.rs        # 改编自上游 figcraft-web：Services 走宿主、暗黑主题、跟随宿主语言
│   │   ├── otools.rs      # wasm-bindgen 桥：openFiles/pickSavePath/writeHostFile/language/theme
│   │   └── download.rs    # 上游浏览器下载回退（原样副本）
│   └── patches/           # 对上游的本地裁剪补丁（随 upstream 重拉而重放）
│       └── figcraft-hostio.patch   # 把 engine 的 wasm 版 read/write 接到宿主桥
├── scripts/               # 构建 / 打包 / 发布 / 验证
└── .github/workflows/     # 独立发布 CI（每个 craft 插件是独立仓库）
```

## 移植要点

### 1. 文件 I/O：同步转宿主桥
上游 `crates/engine/src/cmd/file.rs` 的 `read`/`write` 在 wasm 下原本直接报错
（"no file system in the browser"）。这是完美的补丁切入点：

- `shell/patches/figcraft-hostio.patch` 给 wasm 版 `read` 增加「缓存命中」分支，
  `write` 改成经 `host_io::write` 异步落盘。
- `shell/src/main.rs` 的 `OtoolsServices` 在 `pick_open` 时调用
  `host_io::cache_opened(path, bytes)` 把宿主读到的字节喂给 engine，
  随后 `file.open` 的同步 `read` 才能命中；`pick_save` / `download` 走宿主另存对话框。

### 2. 多语言完整性
FigCraft 自带 11 种语言（含简体中文 `zh-CN`），由上游 i18n 提供。外壳：
- 启动时通过 `window.figcraftOtools.language()` 取宿主语言，写入 `session.prefs.language`；
- 监听 OTools 注入的 `otools-locale-changed` 事件，下一帧重渲染；
- **中文字体**：web 构建必须嵌入 Noto Sans CJK SC（否则中文 UI 显示为方框）。
  由 `CRAFT_FONTS_DIR` + `FIGCRAFT_EMBED_CJK=1` 控制，字体来自兄弟目录 `../craft-fonts`
  （`scripts/fetch-craft-fonts.mjs` 按需浅拉）。

### 3. 默认暗黑主题
`OtoolsServices` 在 `start()` 里：有宿主主题用宿主的，否则 `?theme=` 覆盖，
再否则默认 `"dark"`。egui 的 `prefs.theme` 据此初始化。

### 4. 宿主内聚
吞掉 `OpenUrl` 命令与外部链接（Discord / 文档站），让插件保持独立、不跳出 OTools。

## 本地开发

```bash
# 1) 拉上游 + 拉字体（缺失才拉）
node scripts/fetch-upstream.mjs
node scripts/fetch-craft-fonts.mjs

# 2) 构建 WASM → dist/
node scripts/build-wasm.mjs            # release
node scripts/build-wasm.mjs --debug    # 带调试符号

# 3) 本地预览（可选）
node scripts/serve-dist.mjs

# 4) 同步到 otools 仓库本地试跑
pwsh -File scripts/deploy-to-otools.ps1
```

工具链要求：rustc ≥ 1.95（egui 0.36）、wasm32-unknown-unknown 目标、trunk。

## 发布

独立仓库的 CI 在 `.github/workflows/`：
- `build-plugin.yml`：push/PR 时构建 `dist/` 并上传产物。
- `release-plugin.yml`：打 tag `v*` 或手动触发，构建 + 打包 `.oplg` + 发布到插件市场
  （市场认证走 `secrets.XYCLOUD_PAT`，见 `scripts/publish-market.mjs`）。

最终产物（`plugin.json` + `logo.svg` + `dist/`）由 `deploy-to-otools.ps1`
同步到 otools 主仓库的 `plugins/otools-figcraft/`。
