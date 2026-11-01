// Bundled with the installed DSH shortcut protocol by build-desktop-bridge.mjs.
// Rust MUST inject this only into the trusted DSH origin's top-level frame.
import {
  ShortcutPersistence, initialShortcutConfig, parseShortcutDefinitions,
  parseShortcutEdit, effectiveShortcuts, bindingKey,
} from 'dsh-shortcut-protocol';

(() => {
  if (window.top !== window || window.dshDesktop !== undefined) return;
  const device = navigator.userAgentData?.platform ?? navigator.platform ?? '';
  const platform = /mac|darwin/iu.test(device) ? 'macos' : /win/iu.test(device) ? 'windows' : null;
  if (platform === null || typeof window.__TAURI_INTERNALS__?.invoke !== 'function') return;
  const invoke = (command, args) => window.__TAURI_INTERNALS__.invoke(command, args);
  const detached = (value) => JSON.parse(JSON.stringify(value));
  const keyboardListeners = new Set();
  const configListeners = new Set();
  let definitions = [];
  let snapshot = initialShortcutConfig();
  let bindings = [];
  let recording = false;
  let composing = false;
  let compositionSettling = false;
  let deadKey = false;
  const held = new Set();
  let nativeQueue = Promise.resolve();
  const configureNative = () => {
    const args = { bindings: [...new Set(bindings.map((binding) => bindingKey(binding).toLowerCase()))], recording };
    const next = nativeQueue.then(() => invoke('keyboard_configure', args));
    nativeQueue = next.catch((error) => { console.error('Desktop keyboard configuration failed:', error); });
    return next;
  };
  const reset = () => held.clear();
  const notify = (listeners, value) => {
    for (const listener of [...listeners]) {
      if (!listeners.has(listener)) continue;
      try { listener(detached(value)); }
      catch (error) { console.error('Desktop bridge subscriber failed:', error); }
    }
  };
  const persistence = new ShortcutPersistence({
    async read() {
      const raw = await invoke('read_keybindings');
      if (raw !== null && typeof raw !== 'string') throw new TypeError('Invalid keybindings read response');
      return raw;
    },
    write: (raw) => invoke('write_keybindings', { raw }),
  }, 'desktop', platform, true, (accepted) => {
    snapshot = accepted;
    bindings = accepted.status === 'loading' ? [] : effectiveShortcuts(definitions, accepted.document, 'desktop', platform)
      .filter((row) => row.binding !== null && row.issue === null).map((row) => row.binding);
    reset();
    notify(configListeners, accepted);
    void configureNative().catch(() => {});
  });
  const subscribe = (listeners, listener) => {
    if (typeof listener !== 'function') throw new TypeError('Expected a subscription listener');
    listeners.add(listener);
    return () => listeners.delete(listener);
  };
  const keyboard = Object.freeze({
    subscribe: (listener) => subscribe(keyboardListeners, listener),
    async closeWindow(revision) {
      if (typeof revision !== 'string' || revision !== snapshot.revision || snapshot.status === 'loading') return;
      await invoke('close_shortcut_window');
    },
  });
  const shortcuts = Object.freeze({
    async get(catalog) {
      const validated = parseShortcutDefinitions(detached(catalog));
      definitions = validated;
      persistence.setDefinitions(validated);
      const accepted = await persistence.readCurrent();
      await nativeQueue;
      return detached(accepted);
    },
    async edit(edit, revision) {
      const validated = parseShortcutEdit(detached(edit));
      if (typeof revision !== 'string') throw new TypeError('Invalid shortcut revision');
      const result = await persistence.edit(validated, revision);
      await nativeQueue;
      return detached(result);
    },
    subscribe: (listener) => subscribe(configListeners, listener),
    async recording(active) {
      if (typeof active !== 'boolean') throw new TypeError('Invalid recording state');
      recording = active;
      reset();
      await configureNative();
    },
  });
  // Omit Electron-only browser, updates, deviceInfo and protocolVersion capabilities.
  Object.defineProperty(window, 'dshDesktop', { value: Object.freeze({ keyboard, shortcuts }), enumerable: true });
  const markPlatform = () => {
    if (document.documentElement) document.documentElement.dataset.platform = platform === 'macos' ? 'darwin' : 'win32';
  };
  markPlatform();
  if (!document.documentElement) {
    const observer = new MutationObserver(() => {
      if (!document.documentElement) return;
      markPlatform();
      observer.disconnect();
    });
    observer.observe(document, { childList: true });
  }
  window.addEventListener('compositionstart', () => { composing = true; deadKey = false; reset(); }, true);
  window.addEventListener('compositionend', () => {
    composing = false;
    compositionSettling = true;
    reset();
    setTimeout(() => { compositionSettling = false; }, 0);
  }, true);
  window.addEventListener('blur', () => { composing = false; deadKey = false; reset(); }, true);
  window.addEventListener('focusin', reset, true);
  window.addEventListener('pointerdown', reset, true);
  const modifierCode = /^(Control|Alt|Shift|Meta)(Left|Right)$/u;
  window.addEventListener('keyup', (event) => {
    held.delete(event.code);
    // Some platforms omit keyup for keys released while Command is held.
    if (modifierCode.test(event.code)) reset();
  }, true);
  window.addEventListener('keydown', (event) => {
    if (recording || keyboardListeners.size === 0 || event.defaultPrevented) { reset(); return; }
    const wasDead = deadKey;
    deadKey = event.key === 'Dead';
    if (composing || compositionSettling || event.isComposing || event.keyCode === 229 || wasDead || deadKey || event.key === 'Process' || event.getModifierState?.('AltGraph')) { reset(); return; }
    if (modifierCode.test(event.code) || !event.code) return;
    held.add(event.code);
    if (held.size > 2) return;
    const modifiers = ['control', 'alt', 'shift', 'meta'].filter((name) => event[`${name === 'control' ? 'ctrl' : name}Key`]);
    const codes = [...held].sort();
    const candidate = { code: codes[0], modifiers, ...(codes.length === 2 ? { secondCode: codes[1] } : {}) };
    const key = bindingKey(candidate);
    const binding = bindings.find((value) => bindingKey(value) === key);
    if (binding === undefined) return;
    // Native mode in the product bypasses configurable DOM dispatch. Stop local
    // handlers as well, matching Desktop priority and preventing duplicate actions.
    event.preventDefault();
    event.stopImmediatePropagation();
    notify(keyboardListeners, {
      kind: 'keyboard', frameName: '', revision: snapshot.revision,
      code: binding.code, ...(binding.secondCode === undefined ? {} : { secondCode: binding.secondCode }),
      control: event.ctrlKey, alt: event.altKey, shift: event.shiftKey, meta: event.metaKey,
      repeat: event.repeat,
    });
  }, true);
})();
