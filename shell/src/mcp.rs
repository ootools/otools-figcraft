//! OTools MCP bridge: expose the upstream FigCraft tool layer to the host webview.
//!
//! The host evaluates `window.__otoolsMcp` in this webview. The JavaScript shim calls the
//! two wasm exports below, while the actual tool schemas and implementations stay in the
//! upstream `figcraft-mcp` crate. Calls run against the live `FigApp`, not a second headless
//! session, so agents see and modify the document currently on screen.

use base64::Engine as _;
use figcraft_mcp::{Backend, ToolResult};
use figcraft_ui_egui::FigApp;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

use crate::web::with_live_app;

/// A `Backend` over the web shell's live app and its latest egui context.
struct LiveBackend<'a> {
    app: &'a mut FigApp,
    ctx: &'a egui::Context,
}

impl LiveBackend<'_> {
    fn export_png(&mut self, ids: &[u64], scale: f64) -> Result<Vec<u8>, String> {
        let mut params = json!({"format": "png", "scale": scale});
        if !ids.is_empty() {
            params["ids"] = json!(ids);
        }
        let call = json!({"command": "file.export", "params": params});
        let result =
            figcraft_ui_egui::control::handle(self.app, self.ctx, "engine.execute", &call)?;
        figcraft_mcp::decode_data(&result)
    }

    fn canvas_png(&mut self) -> Result<Vec<u8>, String> {
        let window = web_sys::window().ok_or("no window")?;
        let document = window.document().ok_or("no document")?;
        let canvas = document
            .get_element_by_id(crate::web::CANVAS_ID)
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .ok_or("the FigCraft canvas is not mounted")?;
        let data = canvas
            .to_data_url()
            .map_err(|e| format!("canvas capture failed: {e:?}"))?;
        let encoded = data.strip_prefix("data:image/png;base64,").ok_or_else(|| {
            format!(
                "canvas capture returned an unexpected data URL: {}",
                &data[..data.len().min(80)]
            )
        })?;
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| format!("canvas capture returned invalid base64: {e}"))
    }
}

impl Backend for LiveBackend<'_> {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        figcraft_ui_egui::control::handle(self.app, self.ctx, method, &params)
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        "attached to the FigCraft plugin webview".into()
    }

    fn render_png(&mut self, ids: &[u64], scale: f64) -> Result<Vec<u8>, String> {
        self.export_png(ids, scale)
    }

    fn screenshot_png(&mut self) -> Result<Vec<u8>, String> {
        self.canvas_png()
    }

    fn window_png(&mut self, window: u64) -> Result<Vec<u8>, String> {
        if window != 0 || !self.app.windows.exists(window) {
            return Err(format!(
                "no window {window}; the plugin webview has one window"
            ));
        }
        self.canvas_png()
    }
}

/// The host calls this once to discover every upstream FigCraft tool.
#[wasm_bindgen(js_name = figcraftMcpListTools)]
pub fn list_tools() -> String {
    serde_json::to_string(&figcraft_mcp::tool_definitions()).unwrap_or_else(|e| {
        format!(
            r#"[{{"name":"__error__","description":"{}"}}]"#,
            escape_json(&e.to_string())
        )
    })
}

/// The host calls this for each tool invocation. Every failure is returned in MCP shape.
#[wasm_bindgen(js_name = figcraftMcpCall)]
pub fn call_tool(name: &str, args_json: &str) -> String {
    let args: Value = match serde_json::from_str(args_json) {
        Ok(v) => v,
        Err(e) => {
            return ToolResult::error(format!("invalid arguments JSON: {e}"))
                .to_value()
                .to_string();
        }
    };
    let result = with_live_app(|shell| {
        let Some(ctx) = shell.ctx.clone() else {
            return ToolResult::error(
                "FigCraft is still starting: the UI has not drawn a frame yet",
            );
        };
        let mut backend = LiveBackend {
            app: &mut shell.fig,
            ctx: &ctx,
        };
        figcraft_mcp::call_tool(&mut backend, name, &args)
    })
    .unwrap_or_else(ToolResult::error);
    result.to_value().to_string()
}

fn escape_json(value: &str) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "\"error\"".into())
        .trim_matches('"')
        .to_string()
}
