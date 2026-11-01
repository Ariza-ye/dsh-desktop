//! Local AppKit keyboard forwarding for shortcuts consumed by native menus.
//! Install/uninstall on the main thread; configure only from the trusted main webview.

use serde::Serialize;
use std::{collections::HashMap, sync::Mutex};
use tauri::Manager;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    modifiers: u8,
    key: String,
    companion: Option<String>,
}
const META: u8 = 1;
const CONTROL: u8 = 2;
const ALT: u8 = 4;
const SHIFT: u8 = 8;

impl Binding {
    fn parse(value: &str) -> Result<Self, String> {
        let mut modifiers = 0;
        let mut key = None;
        let mut companion = None;
        for part in value.split('+').map(str::trim) {
            let part = part.to_lowercase();
            match part.as_str() {
                "meta" | "cmd" | "command" | "super" => modifiers |= META,
                "control" | "ctrl" => modifiers |= CONTROL,
                "alt" | "option" => modifiers |= ALT,
                "shift" => modifiers |= SHIFT,
                "" => return Err(format!("Empty key in shortcut {value:?}; use 'plus' for +")),
                _ if key.is_none() => key = Some(normalize_key(&part).to_owned()),
                _ if companion.is_none() => companion = Some(normalize_key(&part).to_owned()),
                _ => return Err(format!("More than two keys in shortcut {value:?}")),
            }
        }
        let key = key.ok_or_else(|| format!("Missing key in shortcut {value:?}"))?;
        if companion.as_ref() == Some(&key) {
            return Err("Chord keys must differ".into());
        }
        Ok(Self {
            modifiers,
            key,
            companion,
        })
    }

    fn matches(&self, event: &DomKey) -> bool {
        let matches_key = |key: &str| {
            key == normalize_key(&event.key.to_lowercase()) || key == event.code.to_lowercase()
        };
        self.modifiers == event.modifiers()
            && (matches_key(&self.key) || self.companion.as_deref().is_some_and(matches_key))
    }
}

fn normalize_key(key: &str) -> &str {
    match key {
        "esc" => "escape",
        "space" | "spacebar" => " ",
        "return" => "enter",
        "plus" => "+",
        "left" => "arrowleft",
        "right" => "arrowright",
        "up" => "arrowup",
        "down" => "arrowdown",
        _ => key,
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DomKey {
    key: String,
    code: String,
    location: u8,
    meta_key: bool,
    ctrl_key: bool,
    alt_key: bool,
    shift_key: bool,
    repeat: bool,
}
impl DomKey {
    fn modifiers(&self) -> u8 {
        u8::from(self.meta_key) * META
            | u8::from(self.ctrl_key) * CONTROL
            | u8::from(self.alt_key) * ALT
            | u8::from(self.shift_key) * SHIFT
    }
}

#[derive(Default)]
struct Configuration {
    bindings: Vec<Binding>,
    recording: bool,
    pressed: HashMap<u16, DomKey>,
}
impl Configuration {
    fn release_command_keys(&mut self) -> Vec<DomKey> {
        let released: Vec<_> = self
            .pressed
            .iter()
            .filter(|(_, key)| key.meta_key)
            .map(|(code, _)| *code)
            .collect();
        released
            .into_iter()
            .filter_map(|code| self.pressed.remove(&code))
            .map(|mut key| {
                key.meta_key = false;
                key.repeat = false;
                key
            })
            .collect()
    }
}
#[derive(Default)]
struct KeyboardState(Mutex<Configuration>);

/// Atomically replace shortcut configuration. Invalid input leaves the old configuration intact.
/// Binding tokens are case-insensitive DOM keys/codes joined with `+`; aliases include
/// cmd/meta, ctrl/control, option/alt, esc/escape, space and plus.
#[tauri::command]
pub fn keyboard_configure(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    bindings: Vec<String>,
    recording: bool,
) -> Result<(), String> {
    if !crate::is_dsh_window(&window) {
        return Err("Keyboard configuration requires the trusted DSH main window".into());
    }
    if bindings.len() > 512 || bindings.iter().any(|binding| binding.len() > 128) {
        return Err("Too many or oversized keyboard bindings".into());
    }
    let bindings = bindings
        .iter()
        .map(|value| Binding::parse(value))
        .collect::<Result<Vec<_>, _>>()?;
    let state = app
        .try_state::<KeyboardState>()
        .ok_or("Keyboard monitor is not installed")?;
    let mut configuration = state.0.lock().map_err(|_| "Keyboard state unavailable")?;
    configuration.bindings = bindings;
    configuration.recording = recording;
    Ok(())
}

/// Hardware-independent DOM `code`, plus the DOM key for non-printable keys.
fn physical_key(code: u16) -> Option<(&'static str, Option<&'static str>, u8)> {
    let (code, key, location) = match code {
        0 => ("KeyA", None, 0),
        1 => ("KeyS", None, 0),
        2 => ("KeyD", None, 0),
        3 => ("KeyF", None, 0),
        4 => ("KeyH", None, 0),
        5 => ("KeyG", None, 0),
        6 => ("KeyZ", None, 0),
        7 => ("KeyX", None, 0),
        8 => ("KeyC", None, 0),
        9 => ("KeyV", None, 0),
        11 => ("KeyB", None, 0),
        12 => ("KeyQ", None, 0),
        13 => ("KeyW", None, 0),
        14 => ("KeyE", None, 0),
        15 => ("KeyR", None, 0),
        16 => ("KeyY", None, 0),
        17 => ("KeyT", None, 0),
        18 => ("Digit1", None, 0),
        19 => ("Digit2", None, 0),
        20 => ("Digit3", None, 0),
        21 => ("Digit4", None, 0),
        22 => ("Digit6", None, 0),
        23 => ("Digit5", None, 0),
        24 => ("Equal", None, 0),
        25 => ("Digit9", None, 0),
        26 => ("Digit7", None, 0),
        27 => ("Minus", None, 0),
        28 => ("Digit8", None, 0),
        29 => ("Digit0", None, 0),
        30 => ("BracketRight", None, 0),
        31 => ("KeyO", None, 0),
        32 => ("KeyU", None, 0),
        33 => ("BracketLeft", None, 0),
        34 => ("KeyI", None, 0),
        35 => ("KeyP", None, 0),
        36 => ("Enter", Some("Enter"), 0),
        37 => ("KeyL", None, 0),
        38 => ("KeyJ", None, 0),
        39 => ("Quote", None, 0),
        40 => ("KeyK", None, 0),
        41 => ("Semicolon", None, 0),
        42 => ("Backslash", None, 0),
        43 => ("Comma", None, 0),
        44 => ("Slash", None, 0),
        45 => ("KeyN", None, 0),
        46 => ("KeyM", None, 0),
        47 => ("Period", None, 0),
        48 => ("Tab", Some("Tab"), 0),
        49 => ("Space", Some(" "), 0),
        50 => ("Backquote", None, 0),
        51 => ("Backspace", Some("Backspace"), 0),
        53 => ("Escape", Some("Escape"), 0),
        54 => ("MetaRight", Some("Meta"), 2),
        55 => ("MetaLeft", Some("Meta"), 1),
        56 => ("ShiftLeft", Some("Shift"), 1),
        57 => ("CapsLock", Some("CapsLock"), 0),
        58 => ("AltLeft", Some("Alt"), 1),
        59 => ("ControlLeft", Some("Control"), 1),
        60 => ("ShiftRight", Some("Shift"), 2),
        61 => ("AltRight", Some("Alt"), 2),
        62 => ("ControlRight", Some("Control"), 2),
        10 => ("IntlBackslash", None, 0),
        65 => ("NumpadDecimal", Some("."), 3),
        67 => ("NumpadMultiply", Some("*"), 3),
        69 => ("NumpadAdd", Some("+"), 3),
        71 => ("NumLock", Some("Clear"), 3),
        75 => ("NumpadDivide", Some("/"), 3),
        76 => ("NumpadEnter", Some("Enter"), 3),
        78 => ("NumpadSubtract", Some("-"), 3),
        81 => ("NumpadEqual", Some("="), 3),
        82 => ("Numpad0", Some("0"), 3),
        83 => ("Numpad1", Some("1"), 3),
        84 => ("Numpad2", Some("2"), 3),
        85 => ("Numpad3", Some("3"), 3),
        86 => ("Numpad4", Some("4"), 3),
        87 => ("Numpad5", Some("5"), 3),
        88 => ("Numpad6", Some("6"), 3),
        89 => ("Numpad7", Some("7"), 3),
        91 => ("Numpad8", Some("8"), 3),
        92 => ("Numpad9", Some("9"), 3),
        93 => ("IntlYen", None, 0),
        94 => ("IntlRo", None, 0),
        102 => ("Lang2", Some("Eisu"), 0),
        104 => ("Lang1", Some("KanjiMode"), 0),
        122 => ("F1", Some("F1"), 0),
        120 => ("F2", Some("F2"), 0),
        99 => ("F3", Some("F3"), 0),
        118 => ("F4", Some("F4"), 0),
        96 => ("F5", Some("F5"), 0),
        97 => ("F6", Some("F6"), 0),
        98 => ("F7", Some("F7"), 0),
        100 => ("F8", Some("F8"), 0),
        101 => ("F9", Some("F9"), 0),
        109 => ("F10", Some("F10"), 0),
        103 => ("F11", Some("F11"), 0),
        111 => ("F12", Some("F12"), 0),
        105 => ("F13", Some("F13"), 0),
        107 => ("F14", Some("F14"), 0),
        113 => ("F15", Some("F15"), 0),
        106 => ("F16", Some("F16"), 0),
        64 => ("F17", Some("F17"), 0),
        79 => ("F18", Some("F18"), 0),
        80 => ("F19", Some("F19"), 0),
        90 => ("F20", Some("F20"), 0),
        114 => ("Insert", Some("Help"), 0),
        115 => ("Home", Some("Home"), 0),
        116 => ("PageUp", Some("PageUp"), 0),
        117 => ("Delete", Some("Delete"), 0),
        119 => ("End", Some("End"), 0),
        121 => ("PageDown", Some("PageDown"), 0),
        123 => ("ArrowLeft", Some("ArrowLeft"), 0),
        124 => ("ArrowRight", Some("ArrowRight"), 0),
        125 => ("ArrowDown", Some("ArrowDown"), 0),
        126 => ("ArrowUp", Some("ArrowUp"), 0),
        _ => return None,
    };
    Some((code, key, location))
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use block2::RcBlock;
    use objc2::{rc::Retained, runtime::AnyObject, MainThreadMarker};
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags as Flags, NSEventType};
    use std::{cell::RefCell, ptr::NonNull};

    type Handler = RcBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent>;
    struct Monitor {
        token: Retained<AnyObject>,
        _handler: Handler,
    }
    impl Drop for Monitor {
        fn drop(&mut self) {
            // Token came from addLocalMonitor; thread-local storage keeps removal on AppKit's thread.
            unsafe { NSEvent::removeMonitor(&self.token) };
        }
    }
    thread_local! { static MONITOR: RefCell<Option<Monitor>> = const { RefCell::new(None) }; }

    fn dom_key(event: &NSEvent) -> Option<DomKey> {
        let (code, special, location) = physical_key(event.keyCode())?;
        let flags = event.modifierFlags();
        let key = match special {
            Some(key) => key.to_owned(),
            None => {
                // Control characters are unsuitable DOM keys; preserve layout while ignoring Ctrl/Cmd.
                let characters = if flags.intersects(Flags::Command | Flags::Control) {
                    event.charactersIgnoringModifiers()
                } else {
                    event.characters()
                }?;
                let key = characters.to_string();
                if key.is_empty() || key.chars().any(char::is_control) {
                    return None;
                }
                key
            }
        };
        Some(DomKey {
            key,
            code: code.into(),
            location,
            meta_key: flags.contains(Flags::Command),
            ctrl_key: flags.contains(Flags::Control),
            alt_key: flags.contains(Flags::Option),
            shift_key: flags.contains(Flags::Shift),
            repeat: event.r#type() != NSEventType::FlagsChanged && event.isARepeat(),
        })
    }

    fn emit(window: &tauri::WebviewWindow, kind: &str, key: &DomKey) -> bool {
        let Ok(origin) = window.url().map(|url| url.origin().ascii_serialization()) else {
            return false;
        };
        let Ok(payload) = serde_json::to_string(key) else {
            return false;
        };
        let Ok(origin) = serde_json::to_string(&origin) else {
            return false;
        };
        // DOM-only events cannot reenter the NSEvent monitor. Check the origin again in case navigation
        // races with the scheduled evaluation. No eval source contains unescaped event characters.
        window.eval(&format!(r#"(()=>{{if(location.origin!=={origin})return;const e=new KeyboardEvent('{kind}',{{...{payload},bubbles:true,cancelable:true,composed:true}});Object.defineProperty(e,'__dshNativeKeyboard',{{value:true}});(document.activeElement||document.body||document).dispatchEvent(e);}})()"#)).is_ok()
    }

    fn has_marked_text(window: &tauri::WebviewWindow) -> bool {
        let Ok(pointer) = window.ns_window() else {
            return false;
        };
        // Called exclusively by AppKit's main-thread local monitor. WKContentView implements
        // NSTextInputClient; check the selector because other first responders need not do so.
        unsafe {
            let native_window = pointer.cast::<AnyObject>();
            let responder: *mut AnyObject = objc2::msg_send![native_window, firstResponder];
            if responder.is_null() {
                return false;
            }
            let supported: bool =
                objc2::msg_send![responder, respondsToSelector: objc2::sel!(hasMarkedText)];
            supported && objc2::msg_send![responder, hasMarkedText]
        }
    }

    fn handle(app: &tauri::AppHandle, event: &NSEvent) -> bool {
        let Some(state) = app.try_state::<KeyboardState>() else {
            return false;
        };
        let Ok(mut configuration) = state.0.lock() else {
            return false;
        };
        let Some(window) = app.get_webview_window("main") else {
            return false;
        };
        if !window.is_focused().unwrap_or(false) || !crate::is_dsh_window(&window) {
            configuration.pressed.clear();
            return false;
        }
        let kind = event.r#type();
        let flags = event.modifierFlags();
        if kind == NSEventType::FlagsChanged {
            // macOS can omit ordinary keyup while Command is down. Release those forwarded keys
            // when Command lifts so JS recorders and pressed-key tracking never stick.
            if !flags.contains(Flags::Command) {
                for key in configuration.release_command_keys() {
                    emit(&window, "keyup", &key);
                }
            }
            return false; // Modifier events still reach AppKit/WKWebView normally.
        }
        if kind == NSEventType::KeyUp {
            if let Some(mut key) = configuration.pressed.remove(&event.keyCode()) {
                key.meta_key = flags.contains(Flags::Command);
                key.ctrl_key = flags.contains(Flags::Control);
                key.alt_key = flags.contains(Flags::Option);
                key.shift_key = flags.contains(Flags::Shift);
                key.repeat = false;
                return emit(&window, "keyup", &key);
            }
            return false;
        }
        if has_marked_text(&window) {
            return false;
        }
        let Some(key) = dom_key(event) else {
            return false;
        };
        // IME mode switches and unknown/dead characters remain native even during recording.
        if matches!(key.code.as_str(), "Lang1" | "Lang2") {
            return false;
        }
        let intercept = configuration.recording
            || configuration
                .bindings
                .iter()
                .any(|binding| binding.matches(&key))
            || configuration.pressed.contains_key(&event.keyCode());
        if intercept && emit(&window, "keydown", &key) {
            configuration.pressed.insert(event.keyCode(), key);
            true
        } else {
            false
        }
    }

    pub fn install(app: &tauri::AppHandle) -> Result<(), String> {
        MainThreadMarker::new().ok_or("Keyboard monitor must be installed on the main thread")?;
        app.manage(KeyboardState::default());
        MONITOR.with(|slot| {
            if slot.borrow().is_some() {
                return Ok(());
            }
            let app = app.clone();
            let handler: Handler = RcBlock::new(move |event: NonNull<NSEvent>| {
                // AppKit guarantees a valid event for this synchronous local monitor callback.
                if handle(&app, unsafe { event.as_ref() }) {
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            });
            // Return either the original live event or null, as required by AppKit.
            let token = unsafe {
                NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                    NSEventMask::KeyDown | NSEventMask::KeyUp | NSEventMask::FlagsChanged,
                    &handler,
                )
            }
            .ok_or("AppKit could not install the local keyboard monitor")?;
            *slot.borrow_mut() = Some(Monitor {
                token,
                _handler: handler,
            });
            Ok(())
        })
    }

    pub fn uninstall() {
        if MainThreadMarker::new().is_some() {
            MONITOR.with(|slot| {
                slot.borrow_mut().take();
            });
        }
    }
}

#[cfg(target_os = "macos")]
pub use native::{install, uninstall};
#[cfg(not(target_os = "macos"))]
pub fn install(app: &tauri::AppHandle) -> Result<(), String> {
    app.manage(KeyboardState::default());
    Ok(())
}
#[cfg(not(target_os = "macos"))]
pub fn uninstall() {}

#[cfg(test)]
mod tests {
    use super::*;
    fn command_n() -> DomKey {
        DomKey {
            key: "n".into(),
            code: "KeyN".into(),
            location: 0,
            meta_key: true,
            ctrl_key: false,
            alt_key: false,
            shift_key: false,
            repeat: false,
        }
    }
    #[test]
    fn maps_physical_codes_and_locations() {
        assert_eq!(physical_key(45), Some(("KeyN", None, 0)));
        assert_eq!(physical_key(55), Some(("MetaLeft", Some("Meta"), 1)));
        assert_eq!(physical_key(54), Some(("MetaRight", Some("Meta"), 2)));
        assert_eq!(physical_key(76), Some(("NumpadEnter", Some("Enter"), 3)));
        assert_eq!(physical_key(123), Some(("ArrowLeft", Some("ArrowLeft"), 0)));
        assert_eq!(physical_key(65535), None);
    }
    #[test]
    fn matches_keys_codes_and_exact_modifiers() {
        let mut event = command_n();
        for binding in ["Cmd+N", "meta+keyn", "COMMAND+n"] {
            assert!(Binding::parse(binding).unwrap().matches(&event));
        }
        assert!(!Binding::parse("ctrl+n").unwrap().matches(&event));
        event.shift_key = true;
        assert!(!Binding::parse("meta+n").unwrap().matches(&event));
        assert!(Binding::parse("shift+meta+n").unwrap().matches(&event));
    }
    #[test]
    fn chord_prefixes_are_forwarded_with_exact_modifiers() {
        let binding = Binding::parse("meta+keya+keyn").unwrap();
        assert!(binding.matches(&command_n()));
        let mut event = command_n();
        event.code = "KeyA".into();
        event.key = "a".into();
        assert!(binding.matches(&event));
        event.code = "KeyB".into();
        event.key = "b".into();
        assert!(!binding.matches(&event));
        event.code = "KeyA".into();
        event.meta_key = false;
        assert!(!binding.matches(&event));
    }
    #[test]
    fn command_release_drains_only_command_keys_once() {
        let mut configuration = Configuration::default();
        let mut command = command_n();
        command.repeat = true;
        configuration.pressed.insert(45, command);
        let mut other = command_n();
        other.meta_key = false;
        other.ctrl_key = true;
        configuration.pressed.insert(0, other);
        let released = configuration.release_command_keys();
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].code, "KeyN");
        assert!(!released[0].meta_key);
        assert!(!released[0].repeat);
        assert!(configuration.pressed.contains_key(&0));
        assert!(configuration.release_command_keys().is_empty());
    }
    #[test]
    fn validates_bindings_and_normalizes_aliases() {
        for invalid in ["", "cmd+", "cmd+n+x+y", "cmd+n+n", "cmd"] {
            assert!(Binding::parse(invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            Binding::parse("ctrl+option+esc").unwrap(),
            Binding::parse("alt+control+escape").unwrap()
        );
        assert_eq!(Binding::parse("meta+plus").unwrap().key, "+");
        assert_eq!(Binding::parse("space").unwrap().key, " ");
    }
}
