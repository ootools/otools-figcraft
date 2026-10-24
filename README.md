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
│   ├── index.html         # window.figcraftOtools 文件桥 + window.__otoolsMcp
│   ├── src/
│   │   ├── main.rs        # 改编自上游 figcraft-web：Services 走宿主、暗黑主题、跟随宿主语言
│   │   ├── mcp.rs         # 复用图上游 figcraft-mcp 的工具表与调用，接到当前 FigApp
│   │   ├── otools.rs      # wasm-bindgen 桥：openFiles/pickSavePath/writeHostFile/language/theme
│   │   └── download.rs    # 上游浏览器下载回退（原样副本）
│   ├── tests/             # MCP webview 桥契约测试（node --test）
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

### 5. MCP：复用上游工具层，操作当前画布
插件通过宿主现有的 **webview MCP 后端**暴露上游 `figcraft-mcp` 的 18 个工具，不另写一套
工具定义或复制 400+ 命令：

- `plugin.json` 声明 `mcp.backend = "webview"`，`expose` 列出全部 18 个工具；
- `shell/index.html` 挂 `window.__otoolsMcp.listTools()/call(name, args)`；
- `shell/src/mcp.rs` 直接调用上游 `tool_definitions()` / `call_tool()`，`Backend` 走
  `figcraft_ui_egui::control::handle`，因此读写的是当前打开的 `FigApp` 与当前文档；
- 工具调用从宿主 eval 进入，`WebRunner::app_mut` 用 `try_borrow_mut`，渲染帧内发生重入时
  会返回可重试的错误，不会二次借用应用状态；
- `screenshot`/`render_png` 在 webview 内通过 canvas/导出生成 PNG，避免依赖临时文件系统。

#### resources 与 prompts 也走同一条上游链路

上游除了 18 个工具，还提供 **2 个资源**（`figcraft://document`、`figcraft://commands`）、
**2 个资源模板**（`figcraft://node/{id}`、`figcraft://command/{id}`）和 **4 个 prompt**
（`mobile_screen` / `design_system` / `landing_page` / `prototype_flow`）。这些同样不复制：

- 桥多暴露 `listResources` / `readResource` / `listResourceTemplates` / `listPrompts` /
  `getPrompt` 五个方法，宿主 `WebviewEval` 逐个调用；
- `shell/src/mcp.rs` 用一个零大小的 `GlobalBackend` 走 `with_live_app` 拿到应用，再交给
  上游 `Server::handle` 分发 `resources/*` 与 `prompts/*`。这样 URI 表、模板的 `{id}` 解析、
  prompt 的默认值替换全部由上游产生，适配层不存在会过期的副本。

首次勾选「暴露工具」时的 seed 由宿主按 `expose` 与 `destructive` 注解决定；会改文档或磁盘的
工具标为 destructive，只读工具标为 readOnly。

#### ES module 打包后导出不在 window 上（0.6.3 修复）

`shell/index.html` 用 `<script type="module">` 加载 wasm，wasm-bindgen 的 ESM 产物把导出
收在命名空间对象里，由 trunk 生成的胶水代码赋给 `window.wasmBindings`，**而不是**逐个挂成
`window.figcraftMcpListTools` 这样的全局。而 MCP 桥原先只认 `window[name]`，于是
`waitForMcpExport` 必然超时（宿主的等待是 5s，插件侧 4.5s），整个 MCP 不可用——0.6.2 发布
包实测就是这个状态。

修复是 `mcpExport(name)`：先看 `window[name]`，再回退 `window.wasmBindings[name]`。
两种加载方式（经典 script 直接挂全局、ES module 挂 bindings）都能工作，并有对应单测。

同时 `plugin.json` 增加 `mcp.webviewUrl = "dist/index.html?webgl"`：宿主（
`src-tauri/src/platform/mcp/webview_lifecycle.rs::plugin_webview_url_for_mode`）优先用这个
URL 加载 webview，只在缺省时回退 `entry`；`?webgl` 让 `shell/src/main.rs` 把 wgpu 后端切成
GL，避免宿主 WebView 里默认后端起不来。

## 上游版本

打包所用的上游提交。`upstream.pin` 是唯一真源，这张表用来回溯「哪个插件版本打的是哪份上游代码」，
每次发布都要在这里加一行。

| 插件版本 | 上游标签 | 上游提交 | 打包日期 |
| --- | --- | --- | --- |
| 0.6.4 | v0.8.0 | `594dc8c6cf50a03b6d88abce877794f286f759b9` | 2026-10-24 |
| 0.6.3 | v0.8.0 | `594dc8c6cf50a03b6d88abce877794f286f759b9` | 2026-10-21 |
| 0.6.2 | v0.8.0 | `594dc8c6cf50a03b6d88abce877794f286f759b9` | 2026-10-21 |
| 0.6.1 | — | `d1d8c3f652cea20bee02bdbc3ded9a2f5362ba08` | — |

## 本地开发

```bash
# 1) 拉上游 + 拉字体（缺失才拉）
node scripts/fetch-upstream.mjs
node scripts/fetch-craft-fonts.mjs

# 2) 构建 WASM → dist/
node scripts/build-wasm.mjs            # release
node scripts/build-wasm.mjs --debug    # 带调试符号

# MCP 桥契约测试
node --test shell/tests/mcp-bridge.test.mjs

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
