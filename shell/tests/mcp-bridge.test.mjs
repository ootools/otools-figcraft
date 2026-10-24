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
    figcraftMcpListResources: options.listResources,
    figcraftMcpReadResource: options.readResource,
    figcraftMcpListResourceTemplates: options.listResourceTemplates,
    figcraftMcpListPrompts: options.listPrompts,
    figcraftMcpGetPrompt: options.getPrompt,
    wasmBindings: options.wasmBindings,
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

test('listTools reads the ES module export object used by the bundled web shell', async () => {
  const tools = [{ name: 'list_commands' }, { name: 'inspect_ui' }];
  const { bridge } = load({
    wasmBindings: { figcraftMcpListTools: () => JSON.stringify(tools) },
  });
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

// ── resources / prompts ────────────────────────────────────────────────────
//
// 上游 figcraft-mcp 提供 2 个资源、2 个资源模板和 4 个 prompt。宿主
// `WebviewEval` 会调 window.__otoolsMcp.listResources / readResource /
// listResourceTemplates / listPrompts / getPrompt，所以这几个方法必须挂在桥上。

test('bridge exposes the resource and prompt methods the host calls', () => {
  const { bridge } = load();
  for (const name of ['listResources', 'readResource', 'listResourceTemplates', 'listPrompts', 'getPrompt']) {
    assert.equal(typeof bridge[name], 'function', `${name} should be on window.__otoolsMcp`);
  }
});

test('listResources returns the upstream resource table', async () => {
  const { bridge } = load({
    listResources: () => JSON.stringify({
      resources: [
        { uri: 'figcraft://document', name: 'document', mimeType: 'application/json' },
        { uri: 'figcraft://commands', name: 'commands', mimeType: 'application/json' },
      ],
    }),
  });
  const listed = await bridge.listResources();
  assert.deepEqual(listed.resources.map((r) => r.uri), [
    'figcraft://document',
    'figcraft://commands',
  ]);
});

test('listResourceTemplates returns the upstream templates', async () => {
  const { bridge } = load({
    listResourceTemplates: () => JSON.stringify({
      resourceTemplates: [
        { uriTemplate: 'figcraft://node/{id}', name: 'node' },
        { uriTemplate: 'figcraft://command/{id}', name: 'command' },
      ],
    }),
  });
  const listed = await bridge.listResourceTemplates();
  assert.equal(listed.resourceTemplates.length, 2);
  assert.equal(listed.resourceTemplates[0].uriTemplate, 'figcraft://node/{id}');
});

test('readResource forwards the uri and returns contents', async () => {
  const seen = [];
  const { bridge } = load({
    readResource: (uri) => {
      seen.push(uri);
      return JSON.stringify({ contents: [{ uri, mimeType: 'application/json', text: '{"id":1}' }] });
    },
  });
  const read = await bridge.readResource('figcraft://node/7');
  assert.deepEqual(seen, ['figcraft://node/7']);
  assert.equal(read.contents[0].text, '{"id":1}');
});

test('readResource surfaces an upstream failure as error, not as empty contents', async () => {
  const { bridge } = load({
    readResource: () => JSON.stringify({ error: 'no such node 7' }),
  });
  const read = await bridge.readResource('figcraft://node/7');
  assert.equal(read.error, 'no such node 7');
  assert.equal(read.contents, undefined);
});

test('listPrompts returns the upstream prompt table', async () => {
  const { bridge } = load({
    listPrompts: () => JSON.stringify({
      prompts: [{ name: 'mobile_screen', title: 'Design a mobile screen', arguments: [] }],
    }),
  });
  const listed = await bridge.listPrompts();
  assert.equal(listed.prompts[0].name, 'mobile_screen');
});

test('getPrompt forwards the name and arguments as JSON text', async () => {
  // wasm 端签名是 (name: &str, args_json: &str)，所以第二个参数是 JSON 字符串。
  const seen = [];
  const { bridge } = load({
    getPrompt: (name, argsJson) => {
      seen.push([name, JSON.parse(argsJson)]);
      return JSON.stringify({
        description: 'one screen',
        messages: [{ role: 'user', content: { type: 'text', text: 'Design a login screen' } }],
      });
    },
  });
  const rendered = await bridge.getPrompt('mobile_screen', { screen: 'login' });
  assert.deepEqual(seen, [['mobile_screen', { screen: 'login' }]]);
  assert.equal(rendered.messages[0].content.text, 'Design a login screen');
});

test('getPrompt with no arguments sends an empty object', async () => {
  const seen = [];
  const { bridge } = load({
    getPrompt: (name, argsJson) => {
      seen.push(JSON.parse(argsJson));
      return JSON.stringify({ messages: [{ role: 'user', content: { type: 'text', text: 'x' } }] });
    },
  });
  await bridge.getPrompt('design_system');
  assert.deepEqual(seen, [{}]);
});

test('a missing resource export degrades to an empty table instead of hanging', async () => {
  // 老版本 wasm 没有这些导出：宿主按「该插件没有资源」处理，不能让桥挂住。
  // 注意用长度而不是 deepEqual：桥跑在 vm realm 里，数组不是同一个 Array 原型。
  const { bridge } = load();
  const listed = await bridge.listResources();
  assert.equal(listed.resources.length, 0);
  const templates = await bridge.listResourceTemplates();
  assert.equal(templates.resourceTemplates.length, 0);
  const prompts = await bridge.listPrompts();
  assert.equal(prompts.prompts.length, 0);
});

test('plugin manifest declares the upstream resources and prompts', () => {
  const manifest = JSON.parse(fs.readFileSync(path.resolve(shell, '..', 'plugin.json'), 'utf8'));
  assert.deepEqual(manifest.mcp.resources, ['figcraft://document', 'figcraft://commands']);
  assert.deepEqual(manifest.mcp.prompts, [
    'mobile_screen',
    'design_system',
    'landing_page',
    'prototype_flow',
  ]);
});
