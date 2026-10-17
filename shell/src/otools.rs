//! OTools 宿主桥：把「文件从哪来、存到哪去」换成 OTools 宿主的对话框与磁盘读写。
//!
//! 上游 `apps/figcraft-web` 在浏览器里没有文件系统：打开靠 `<input type=file>`，保存靠浏览器
//! 下载。OTools 宿主注入的 `window.otools` 提供了真正的系统文件对话框与 `readHostFile` /
//! `writeHostFile`（底层是宿主的 `tools_webview_read_file` / `tools_webview_write_file`，
//! 无路径白名单限制），所以本模块是插件**唯一**需要新写的 Rust 代码；engine 的 host_io
//! 补丁（`shell/patches/figcraft-hostio.patch`）把同步的 `read`/`write` 接到这里。
//!
//! 与上游的对接点：
//! - `Services::pick_open` → 弹宿主打开对话框，字节经 `host_io::cache_opened` 喂给 engine。
//! - `Services::pick_save` → 弹宿主另存对话框，返回路径；engine 的 `write` 经 `writeHostFile` 落盘。
//! - `Services::download` → 导出（PNG/SVG/PDF）经宿主另存对话框落盘。
//!
//! JS 侧的实现见 `shell/index.html` 里的 `window.figcraftOtools`。

use std::path::PathBuf;

use base64::Engine as _;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::js_sys;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = figcraftOtools, js_name = "available")]
    fn host_available() -> bool;

    #[wasm_bindgen(js_namespace = figcraftOtools, js_name = "theme")]
    fn host_theme() -> String;

    /// 宿主界面语言（BCP 47，如 `zh-CN`）：OTools 注入到插件 webview 的
    /// `window.__OTOOLS_LOCALE__.locale` / `document.documentElement.lang`。
    #[wasm_bindgen(js_namespace = figcraftOtools, js_name = "language")]
    fn host_language_raw() -> String;

    /// 弹打开文件对话框并读取选中文件，解析成 `[{name, path, dataBase64}]`。
    #[wasm_bindgen(js_namespace = figcraftOtools, js_name = "openFiles")]
    fn host_open_files() -> js_sys::Promise;

    /// 弹另存对话框，返回选中的路径（`null` = 用户取消）。
    #[wasm_bindgen(js_namespace = figcraftOtools, js_name = "pickSavePath")]
    fn host_pick_save_path(name: &str) -> js_sys::Promise;

    /// 写宿主磁盘文件：`{path, dataBase64}`（engine 的 host_io 与导出下载都走这条）。
    #[wasm_bindgen(js_namespace = figcraftOtools, js_name = "writeHostFile")]
    fn host_write_file(path: &str, data_base64: &str) -> js_sys::Promise;
}

/// 宿主是否可用（不在 OTools 里时退回上游的浏览器行为：下载文件）。
pub fn available() -> bool {
    host_available()
}

/// OTools 的界面主题，用于给 egui 一个初始主题（`""` = 跟随系统）。
pub fn theme() -> String {
    host_theme()
}

/// 宿主界面语言（BCP 47，如 `zh-CN`）：跟随 OTools 宿主语言切换。
pub fn language() -> String {
    host_language_raw()
}

/// 打开文件：弹宿主对话框，读取字节，返回 `(路径, 字节)`。取消返回 `None`。
pub async fn pick_open() -> Option<(String, Vec<u8>)> {
    let value = JsFuture::from(host_open_files()).await.ok()?;
    let entry = js_sys::Array::from(&value).get(0);
    if entry.is_undefined() {
        return None;
    }
    let name = field(&entry, "name").unwrap_or_else(|| "design.figcraft".into());
    let path = field(&entry, "path").unwrap_or_else(|| name.clone());
    let data = field(&entry, "dataBase64").unwrap_or_default();
    let bytes = base64::engine::general_purpose::STANDARD.decode(data).ok()?;
    Some((path, bytes))
}

/// 另存为：弹宿主对话框，返回选中的路径；取消返回 `None`。
pub async fn pick_save_path(name: &str) -> Option<String> {
    let value = JsFuture::from(host_pick_save_path(name)).await.ok()?;
    if value.is_null() || value.is_undefined() {
        return None;
    }
    value.as_string()
}

/// 导出下载：弹宿主另存对话框并写盘，返回是否成功。
pub async fn save_download(name: &str, bytes: &[u8]) -> bool {
    let Some(path) = pick_save_path(name).await else {
        return false;
    };
    let data_base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    let _ = JsFuture::from(host_write_file(&path, &data_base64)).await;
    true
}

/// 取对象上的字符串字段。
fn field(value: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(value, &JsValue::from_str(key)).ok()?.as_string()
}
