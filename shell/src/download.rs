//! 【OTools 移植】本文件是上游 `apps/figcraft-web/src/web.rs` 里 `download` 的**原样副本**，请勿就地修改。
//! 上游更新时把同名函数重新复制过来即可；需要改的行为请放进 `otools.rs` / `main.rs`。
//! Browser downloads: hand bytes to the user as a file, through web-sys only (a `Blob`, an object
//! URL and a temporary `<a download>` link). This is the web's file route for everything that
//! produces a file: exports today (installed as the `Services::download` hook in `main.rs`).

use wasm_bindgen::JsCast as _;
use web_sys::js_sys;

/// How long the object URL stays valid after the click; browsers read it asynchronously.
const REVOKE_DOWNLOAD_MS: i32 = 40_000;

/// Start a browser download of `bytes` as a file called `name`.
///
/// `Ok` once the download was triggered. Where it lands (or whether the user cancels a "save as"
/// prompt of the browser) is up to the browser and can't be observed from the page.
pub fn download(name: &str, bytes: &[u8]) -> Result<(), wasm_bindgen::JsValue> {
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let body = document.body().ok_or("no body")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &{
        let o = web_sys::BlobPropertyBag::new();
        o.set_type("application/octet-stream");
        o
    })?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a")?.dyn_into()?;
    a.set_href(&url);
    a.set_download(name);
    body.append_child(&a)?;
    a.click();
    a.remove();
    // Revoked later, not at once: some browsers cancel a download whose URL goes before it starts.
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    // Without the timer the URL just stays until the page closes.
    let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), REVOKE_DOWNLOAD_MS);
    Ok(())
}
