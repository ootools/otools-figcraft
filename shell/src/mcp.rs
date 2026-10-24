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

/// A `Backend` that reaches the live app through the same thread-local `with_live_app`
/// uses, so it can be boxed as `dyn Backend + 'static`.
///
/// Why this exists: upstream keeps the whole `resources/*` and `prompts/*` dispatch inside
/// `Server::handle`, and `Server::new` takes `Box<dyn Backend>` by value. The live app is
/// only reachable through a borrow, which is not `'static` — so instead of copying the
/// resource URIs and prompt templates into this shell (they would drift), we hand the
/// server a zero-sized backend that looks the app up per call. A `Server` is then created
/// per request and thrown away; nothing stateful lives in it.
struct GlobalBackend;

impl Backend for GlobalBackend {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        with_live_app(|shell| match shell.ctx.clone() {
            Some(ctx) => figcraft_ui_egui::control::handle(&mut shell.fig, &ctx, method, &params),
            None => Err("FigCraft is still starting: the UI has not drawn a frame yet".to_string()),
        })?
    }

    fn has_ui(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        "attached to the FigCraft plugin webview".into()
    }
}

/// Run one upstream MCP method and take its `result`.
fn rpc(method: &str, params: Value) -> Result<Value, String> {
    let mut server = figcraft_mcp::Server::new(Box::new(GlobalBackend));
    let reply = server
        .handle(json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .ok_or_else(|| format!("upstream did not answer `{method}`"))?;
    if let Some(error) = reply.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("upstream MCP error");
        return Err(message.to_string());
    }
    reply
        .get("result")
        .cloned()
        .ok_or_else(|| format!("`{method}` returned no result"))
}

/// Serialize a result for the JS bridge; failures travel as `{ error }`, which is what the
/// host's webview backend looks for.
fn bridge_json(value: Result<Value, String>) -> String {
    match value {
        Ok(value) => value.to_string(),
        Err(error) => json!({ "error": error }).to_string(),
    }
}

/// `resources/list`: the active document and the command catalog.
#[wasm_bindgen(js_name = figcraftMcpListResources)]
pub fn list_resources() -> String {
    bridge_json(rpc("resources/list", json!({})))
}

/// `resources/templates/list`: one layer (`figcraft://node/{id}`) and one command
/// (`figcraft://command/{id}`).
#[wasm_bindgen(js_name = figcraftMcpListResourceTemplates)]
pub fn list_resource_templates() -> String {
    bridge_json(rpc("resources/templates/list", json!({})))
}

/// `resources/read`. Template variables are decoded upstream, so a misspelt id comes back
/// as the upstream "no such node" message rather than silently empty content.
#[wasm_bindgen(js_name = figcraftMcpReadResource)]
pub fn read_resource(uri: &str) -> String {
    if uri.trim().is_empty() {
        return bridge_json(Err("missing resource uri".into()));
    }
    bridge_json(rpc("resources/read", json!({ "uri": uri })))
}

/// `prompts/list`: the ready-made design workflows.
#[wasm_bindgen(js_name = figcraftMcpListPrompts)]
pub fn list_prompts() -> String {
    bridge_json(rpc("prompts/list", json!({})))
}

/// `prompts/get`: render one workflow with its arguments and defaults.
#[wasm_bindgen(js_name = figcraftMcpGetPrompt)]
pub fn get_prompt(name: &str, args_json: &str) -> String {
    if name.trim().is_empty() {
        return bridge_json(Err("missing prompt name".into()));
    }
    let arguments: Value = match serde_json::from_str(args_json) {
        Ok(value) => value,
        Err(error) => return bridge_json(Err(format!("invalid arguments JSON: {error}"))),
    };
    bridge_json(rpc(
        "prompts/get",
        json!({ "name": name, "arguments": arguments }),
    ))
}
