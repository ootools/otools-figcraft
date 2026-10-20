import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import vm from 'node:vm';
import { fileURLToPath } from 'node:url';

const shell = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const html = fs.readFileSync(path.join(shell, 'index.html'), 'utf8');

function bridgeSource() {
  const startMarker = '// <otools-mcp-bridge>';
  const endMarker = '// </otools-mcp-bridge>';
  const start = html.indexOf(startMarker);
  assert.notEqual(start, -1, 'index.html should mark the MCP bridge');
  const end = html.indexOf(endMarker, start);
  assert.notEqual(end, -1, 'index.html should close the MCP bridge marker');
  return html.slice(start + startMarker.length, end);
}

function load(options = {}) {
  const calls = [];
  const context = {
    console,
    JSON,
    String,
    Error,
    Promise,
    Date,
    setTimeout,
    clearTimeout,
    __OTOOLS_MCP_READY_TIMEOUT_MS: options.readyTimeoutMs ?? 500,
    figcraftMcpListTools: options.listTools,
    figcraftMcpCall: options.call,
  };
  context.window = context;
  vm.createContext(context);
  vm.runInContext(bridgeSource(), context);
  return { bridge: context.__otoolsMcp, calls, context };
}

test('bridge exposes listTools and call', () => {
  const { bridge } = load();
  assert.equal(typeof bridge.listTools, 'function');
  assert.equal(typeof bridge.call, 'function');
});

test('listTools reads the upstream report and returns an array', async () => {
  const tools = [{ name: 'list_commands' }, { name: 'run_command' }];
  const { bridge } = load({ listTools: () => JSON.stringify(tools) });
  assert.deepEqual(await bridge.listTools(), tools);
});

test('call forwards the tool name and parsed arguments, and returns MCP shape', async () => {
  const seen = [];
  const result = { content: [{ type: 'text', text: 'ok' }], isError: false };
  const { bridge } = load({
    call: (name, args) => {
      seen.push([name, args]);
      return JSON.stringify(result);
    },
  });
  assert.deepEqual(await bridge.call('run_command', { command: 'select.none', params: {} }), result);
  assert.deepEqual(seen, [['run_command', '{"command":"select.none","params":{}}']]);
});

test('listTools waits for a WASM export that mounts after the HTML bridge', async () => {
  const tools = [{ name: 'run_command' }];
  const { bridge, context } = load({ readyTimeoutMs: 500 });
  setTimeout(() => {
    context.figcraftMcpListTools = () => JSON.stringify(tools);
  }, 25);
  assert.deepEqual(await bridge.listTools(), tools);
});

test('a missing Rust export is reported as an MCP tool error', async () => {
  const { bridge } = load({ readyTimeoutMs: 0 });
  const list = await bridge.listTools();
  assert.equal(Array.isArray(list), true);
  assert.equal(list.length, 0);
  const result = await bridge.call('run_command', {});
  assert.equal(result.isError, true);
  assert.match(result.content[0].text, /figcraftMcpCall/);
});

test('a thrown Rust error becomes isError instead of escaping', async () => {
  const { bridge } = load({
    call: () => {
      throw new Error('tool exploded');
    },
  });
  const result = await bridge.call('run_command', {});
  assert.equal(result.isError, true);
  assert.match(result.content[0].text, /tool exploded/);
});


test('plugin manifest advertises the webview bridge and all 18 upstream tools', () => {
  const manifest = JSON.parse(fs.readFileSync(path.resolve(shell, '..', 'plugin.json'), 'utf8'));
  assert.equal(manifest.mcp.enabled, true);
  assert.equal(manifest.mcp.backend, 'webview');
  assert.equal(manifest.mcp.namespace, 'figcraft');
  assert.deepEqual(manifest.mcp.expose, [
    'list_commands',
    'run_command',
    'batch',
    'create_node',
    'set_properties',
    'inspect_document',
    'get_node',
    'select',
    'export',
    'screenshot',
    'open_file',
    'save_file',
    'undo',
    'redo',
    'pointer',
    'key',
    'type_text',
    'inspect_ui',
  ]);
});

test('plugin manifest marks tools that can change the document or disk as destructive', () => {
  const manifest = JSON.parse(fs.readFileSync(path.resolve(shell, '..', 'plugin.json'), 'utf8'));
  for (const name of ['run_command', 'batch', 'create_node', 'set_properties', 'save_file', 'undo', 'redo', 'pointer', 'key', 'type_text']) {
    assert.equal(manifest.mcp.annotations?.[name]?.destructive, true, `${name} should be destructive`);
  }
  for (const name of ['list_commands', 'inspect_document', 'get_node', 'screenshot', 'inspect_ui']) {
    assert.equal(manifest.mcp.annotations?.[name]?.readOnly, true, `${name} should be read-only`);
  }
});
