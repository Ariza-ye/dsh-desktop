import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { webcrypto } from 'node:crypto';
import vm from 'node:vm';

const source = await readFile(new URL('../src-tauri/src/desktop-bridge.js', import.meta.url), 'utf8');
function setup(platform = 'MacIntel', topFrame = true) {
  const listeners = new Map();
  const calls = [];
  let raw = null;
  let readFails = false;
  let writeFails = false;
  const window = {
    addEventListener(type, listener) { const list = listeners.get(type) ?? []; list.push(listener); listeners.set(type, list); },
    __TAURI_INTERNALS__: { async invoke(command, args) {
      calls.push({ command, args });
      if (command === 'read_keybindings') { if (readFails) throw new Error('read'); return raw; }
      if (command === 'write_keybindings') { if (writeFails) throw new Error('write'); raw = args.raw; }
    } },
  };
  window.top = topFrame ? window : {};
  const document = { documentElement: { dataset: {} } };
  vm.runInNewContext(source, { window, document, navigator: { platform }, crypto: webcrypto, console, setTimeout });
  const fire = (type, input = {}) => {
    const event = { code: 'KeyN', key: 'n', ctrlKey: false, altKey: false, shiftKey: false, metaKey: true, repeat: false, defaultPrevented: false,
      getModifierState: () => false,
      preventDefault() { this.defaultPrevented = true; },
      stopImmediatePropagation() { this.stopped = true; }, ...input };
    for (const listener of listeners.get(type) ?? []) { listener(event); if (event.stopped) break; }
    return event;
  };
  return { bridge: window.dshDesktop, document, calls, fire,
    setRaw(value) { raw = value; }, getRaw() { return raw; },
    readFail(value) { readFails = value; }, writeFail(value) { writeFails = value; } };
}
test('desktop bridge persistence and keyboard contract in a browser VM', async () => {
const catalog = [
  { id: 'app.new', defaults: { 'desktop:macos': { code: 'KeyN', modifiers: ['primary'] }, 'desktop:windows': { code: 'KeyN', modifiers: ['primary'] } } },
  { id: 'app.other', defaults: { 'desktop:macos': { code: 'KeyO', modifiers: ['primary'] } } },
  { id: 'app.fixed', defaults: {}, fixed: [{ code: 'Enter', modifiers: [] }] },
];
const environment = setup();
const { bridge, fire, calls } = environment;
assert.equal(environment.document.documentElement.dataset.platform, 'darwin');
assert.deepEqual(Object.keys(bridge).sort(), ['keyboard', 'shortcuts']);
assert.equal(setup('Linux x86_64').bridge, undefined);
assert.equal(setup('MacIntel', false).bridge, undefined);
const windows = setup('Win32');
assert.equal(windows.document.documentElement.dataset.platform, 'win32');
await windows.bridge.shortcuts.get(catalog);
const windowsInputs = [];
windows.bridge.keyboard.subscribe((value) => windowsInputs.push(value));
assert.equal(windows.fire('keydown', { ctrlKey: true, metaKey: false }).defaultPrevented, true);
assert.equal(windowsInputs[0].control, true);
const snapshots = [];
const unsubscribe = bridge.shortcuts.subscribe((value) => snapshots.push(value));
let snapshot = await bridge.shortcuts.get(catalog);
assert.equal(snapshot.status, 'ready');
assert.equal(snapshot.usingDefaults, true);
assert.ok(snapshot.revision);
assert.ok(snapshots.length >= 2);
const inputs = [];
const off = bridge.keyboard.subscribe((value) => inputs.push(value));
let event = fire('keydown');
assert.equal(event.defaultPrevented, true);
assert.equal(event.stopped, true);
assert.equal(inputs.length, 1);
assert.equal(inputs[0].revision, snapshot.revision);
assert.equal(inputs[0].kind, 'keyboard');
fire('keydown', { repeat: true });
assert.equal(inputs.at(-1).repeat, true); // Registry consumes repeats without running the command.
fire('keyup');
fire('keydown', { isComposing: true });
fire('keydown', { keyCode: 229 });
fire('keydown', { key: 'Dead' });
fire('keydown'); // The next key completes the dead-key sequence.
assert.equal(inputs.length, 2);
fire('compositionstart');
fire('keydown');
assert.equal(inputs.length, 2);
fire('compositionend');
await new Promise((resolve) => setTimeout(resolve, 1));
await bridge.shortcuts.recording(true);
assert.equal(calls.at(-1).command, 'keyboard_configure');
assert.equal(calls.at(-1).args.recording, true);
event = fire('keydown');
assert.equal(event.defaultPrevented, false);
assert.equal(inputs.length, 2);
await bridge.shortcuts.recording(false);
let result = await bridge.shortcuts.edit({ type: 'set', id: 'app.new', binding: { code: 'KeyB', secondCode: 'KeyA', modifiers: ['primary'] } }, snapshot.revision);
assert.equal(result.status, 'saved');
snapshot = result.snapshot;
assert.equal(snapshot.document.schemaVersion, 2);
assert.equal(snapshot.document.profiles['desktop:macos']['app.new'].secondCode, 'KeyA');
fire('keydown', { code: 'KeyA' });
event = fire('keydown', { code: 'KeyB' });
assert.equal(event.defaultPrevented, true);
assert.equal(inputs.at(-1).code, 'KeyA');
assert.equal(inputs.at(-1).secondCode, 'KeyB');
assert.equal(inputs.at(-1).revision, snapshot.revision);
fire('blur');
result = await bridge.shortcuts.edit({ type: 'set', id: 'app.new', binding: { code: 'KeyO', modifiers: ['meta'] } }, snapshot.revision);
assert.equal(result.status, 'conflict');
assert.ok(result.conflicts.includes('app.other'));
result = await bridge.shortcuts.edit({ type: 'set', id: 'app.new', binding: { code: 'Enter', modifiers: [] } }, snapshot.revision);
assert.equal(result.status, 'conflict');
assert.ok(result.conflicts.includes('app.fixed'));
result = await bridge.shortcuts.edit({ type: 'reset-all' }, 'old-revision');
assert.equal(result.status, 'stale');
await assert.rejects(bridge.shortcuts.edit({ type: 'set', id: 'app.new', binding: { code: 'Bogus', modifiers: [] } }, snapshot.revision));
await assert.rejects(bridge.shortcuts.get([...catalog, catalog[0]]));
await assert.rejects(bridge.shortcuts.recording('true'));
environment.writeFail(true);
result = await bridge.shortcuts.edit({ type: 'reset', id: 'app.new' }, snapshot.revision);
assert.equal(result.status, 'write-failed');
assert.equal(result.snapshot.revision, snapshot.revision);
environment.writeFail(false);
environment.readFail(true);
result = await bridge.shortcuts.edit({ type: 'reset-all' }, snapshot.revision);
assert.equal(result.status, 'stale'); // Reread diagnostic changes revision first.
snapshot = result.snapshot;
assert.equal(snapshot.status, 'unreadable');
assert.equal(snapshot.error, 'read');
result = await bridge.shortcuts.edit({ type: 'reset-all' }, snapshot.revision);
assert.equal(result.status, 'unreadable');
environment.readFail(false);
environment.setRaw('{broken');
snapshot = await bridge.shortcuts.get(catalog);
assert.equal(snapshot.error, 'invalid');
assert.equal(snapshot.document.schemaVersion, 2); // Last accepted preferences retained.
environment.setRaw('{"schemaVersion":99,"profiles":{}}');
snapshot = await bridge.shortcuts.get(catalog);
assert.equal(snapshot.error, 'future');
environment.setRaw(JSON.stringify({ schemaVersion: 1, profiles: { 'web:linux': { 'dormant.command': null } } }));
snapshot = await bridge.shortcuts.get(catalog);
result = await bridge.shortcuts.edit({ type: 'reset-all' }, snapshot.revision);
assert.equal(result.status, 'saved');
assert.equal(result.snapshot.document.profiles['web:linux']['dormant.command'], null);
const closeBefore = calls.filter((value) => value.command === 'close_shortcut_window').length;
await bridge.keyboard.closeWindow(snapshot.revision);
assert.equal(calls.filter((value) => value.command === 'close_shortcut_window').length, closeBefore);
await bridge.keyboard.closeWindow(result.snapshot.revision);
assert.equal(calls.at(-1).command, 'close_shortcut_window');
// Subscribers cannot mutate the coordinator's accepted document or each other's values.
const mutable = await bridge.shortcuts.get(catalog);
mutable.document.profiles['desktop:macos']['app.new'] = null;
const fresh = await bridge.shortcuts.get(catalog);
assert.equal(Object.hasOwn(fresh.document.profiles['desktop:macos'], 'app.new'), false);
unsubscribe();
off();
});
