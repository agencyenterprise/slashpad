//! Global hotkey registration.
//!
//! The `global-hotkey` crate fires events on a platform event loop. We poll
//! its receiver from a blocking tokio task and forward presses into an
//! unbounded channel that iced drains via a `Subscription`.

use std::sync::OnceLock;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager};
use tokio::sync::mpsc;

/// The sender half of the channel that delivers HotkeyPressed events into
/// the iced subscription. Set once in `spawn`.
static HOTKEY_TX: OnceLock<mpsc::UnboundedSender<()>> = OnceLock::new();

/// Set by `spawn` and used by `update_hotkey` to swap the registered chord.
static HOTKEY_MANAGER: OnceLock<GlobalHotKeyManager> = OnceLock::new();
static CURRENT_HOTKEY: std::sync::Mutex<Option<HotKey>> = std::sync::Mutex::new(None);

#[derive(Debug, thiserror::Error)]
pub enum HotkeyError {
    #[error("invalid shortcut: {0}")]
    Parse(String),
    #[error("register failed: {0}")]
    Register(String),
    #[error("manager not initialized")]
    NotInitialized,
}

/// Parse a shortcut string like `Ctrl+Space`, `Cmd+Shift+K`, `Super+Alt+/`
/// into a `HotKey`. Mirrors the format `HotkeyRecorder.tsx` emits.
pub fn parse_hotkey(s: &str) -> Result<HotKey, HotkeyError> {
    let parts: Vec<&str> = s.split('+').map(str::trim).collect();
    if parts.is_empty() {
        return Err(HotkeyError::Parse(s.to_string()));
    }
    let mut mods = Modifiers::empty();
    let mut key_name: Option<&str> = None;
    for part in &parts {
        match *part {
            "Ctrl" | "Control" => mods |= Modifiers::CONTROL,
            "Alt" | "Option" => mods |= Modifiers::ALT,
            "Shift" => mods |= Modifiers::SHIFT,
            "Super" | "Cmd" | "Command" | "Meta" => mods |= Modifiers::SUPER,
            other => key_name = Some(other),
        }
    }
    let key = key_name.ok_or_else(|| HotkeyError::Parse(s.to_string()))?;
    let code = keyname_to_code(key).ok_or_else(|| HotkeyError::Parse(s.to_string()))?;
    Ok(HotKey::new(Some(mods), code))
}

fn keyname_to_code(key: &str) -> Option<Code> {
    // Letters
    if key.len() == 1 {
        let c = key.chars().next().unwrap().to_ascii_uppercase();
        return match c {
            'A' => Some(Code::KeyA),
            'B' => Some(Code::KeyB),
            'C' => Some(Code::KeyC),
            'D' => Some(Code::KeyD),
            'E' => Some(Code::KeyE),
            'F' => Some(Code::KeyF),
            'G' => Some(Code::KeyG),
            'H' => Some(Code::KeyH),
            'I' => Some(Code::KeyI),
            'J' => Some(Code::KeyJ),
            'K' => Some(Code::KeyK),
            'L' => Some(Code::KeyL),
            'M' => Some(Code::KeyM),
            'N' => Some(Code::KeyN),
            'O' => Some(Code::KeyO),
            'P' => Some(Code::KeyP),
            'Q' => Some(Code::KeyQ),
            'R' => Some(Code::KeyR),
            'S' => Some(Code::KeyS),
            'T' => Some(Code::KeyT),
            'U' => Some(Code::KeyU),
            'V' => Some(Code::KeyV),
            'W' => Some(Code::KeyW),
            'X' => Some(Code::KeyX),
            'Y' => Some(Code::KeyY),
            'Z' => Some(Code::KeyZ),
            '0' => Some(Code::Digit0),
            '1' => Some(Code::Digit1),
            '2' => Some(Code::Digit2),
            '3' => Some(Code::Digit3),
            '4' => Some(Code::Digit4),
            '5' => Some(Code::Digit5),
            '6' => Some(Code::Digit6),
            '7' => Some(Code::Digit7),
            '8' => Some(Code::Digit8),
            '9' => Some(Code::Digit9),
            '/' => Some(Code::Slash),
            '-' => Some(Code::Minus),
            '=' => Some(Code::Equal),
            '[' => Some(Code::BracketLeft),
            ']' => Some(Code::BracketRight),
            '\\' => Some(Code::Backslash),
            ';' => Some(Code::Semicolon),
            '\'' => Some(Code::Quote),
            ',' => Some(Code::Comma),
            '.' => Some(Code::Period),
            '`' => Some(Code::Backquote),
            _ => None,
        };
    }

    match key {
        "Space" => Some(Code::Space),
        "Enter" | "Return" => Some(Code::Enter),
        "Tab" => Some(Code::Tab),
        "Backspace" => Some(Code::Backspace),
        "Delete" => Some(Code::Delete),
        "Escape" | "Esc" => Some(Code::Escape),
        "Up" => Some(Code::ArrowUp),
        "Down" => Some(Code::ArrowDown),
        "Left" => Some(Code::ArrowLeft),
        "Right" => Some(Code::ArrowRight),
        "F1" => Some(Code::F1),
        "F2" => Some(Code::F2),
        "F3" => Some(Code::F3),
        "F4" => Some(Code::F4),
        "F5" => Some(Code::F5),
        "F6" => Some(Code::F6),
        "F7" => Some(Code::F7),
        "F8" => Some(Code::F8),
        "F9" => Some(Code::F9),
        "F10" => Some(Code::F10),
        "F11" => Some(Code::F11),
        "F12" => Some(Code::F12),
        _ => None,
    }
}

/// Registers the hotkey manager and the given shortcut, then spawns a tokio
/// blocking task that polls for events and forwards them to the returned
/// receiver. Call once at startup.
pub fn spawn(initial_shortcut: &str) -> Result<mpsc::UnboundedReceiver<()>, HotkeyError> {
    let manager =
        GlobalHotKeyManager::new().map_err(|e| HotkeyError::Register(e.to_string()))?;
    let hotkey = parse_hotkey(initial_shortcut)?;
    manager
        .register(hotkey)
        .map_err(|e| HotkeyError::Register(e.to_string()))?;

    *CURRENT_HOTKEY.lock().unwrap() = Some(hotkey);
    let _ = HOTKEY_MANAGER.set(manager);

    let (tx, rx) = mpsc::unbounded_channel::<()>();
    let _ = HOTKEY_TX.set(tx.clone());

    // Poller task: drain global-hotkey events on a blocking thread and forward.
    std::thread::spawn(move || {
        let receiver = GlobalHotKeyEvent::receiver();
        while let Ok(event) = receiver.recv() {
            if event.state == global_hotkey::HotKeyState::Pressed && tx.send(()).is_err() {
                break;
            }
        }
    });

    Ok(rx)
}

/// Build a canonical chord string (parseable by `parse_hotkey`) from an iced
/// key event. Returns `None` for modifier-only presses — callers should keep
/// recording until a "real" key arrives.
///
/// Modifier order is fixed (`Ctrl+Alt+Shift+Cmd+<key>`) so the same chord
/// always serializes the same way.
pub fn format_chord(
    key: &iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Option<String> {
    use iced::keyboard::key::Named;
    use iced::keyboard::Key;

    let key_token: String = match key {
        Key::Named(named) => match named {
            // Skip modifier-only presses — wait for a real key.
            Named::Shift | Named::Control | Named::Alt | Named::Super | Named::Meta => {
                return None
            }
            Named::Space => "Space".into(),
            Named::Enter => "Enter".into(),
            Named::Tab => "Tab".into(),
            Named::Backspace => "Backspace".into(),
            Named::Delete => "Delete".into(),
            Named::Escape => "Escape".into(),
            Named::ArrowUp => "Up".into(),
            Named::ArrowDown => "Down".into(),
            Named::ArrowLeft => "Left".into(),
            Named::ArrowRight => "Right".into(),
            Named::F1 => "F1".into(),
            Named::F2 => "F2".into(),
            Named::F3 => "F3".into(),
            Named::F4 => "F4".into(),
            Named::F5 => "F5".into(),
            Named::F6 => "F6".into(),
            Named::F7 => "F7".into(),
            Named::F8 => "F8".into(),
            Named::F9 => "F9".into(),
            Named::F10 => "F10".into(),
            Named::F11 => "F11".into(),
            Named::F12 => "F12".into(),
            _ => return None,
        },
        Key::Character(c) => {
            let s = c.as_str();
            if s.is_empty() {
                return None;
            }
            s.to_ascii_uppercase()
        }
        Key::Unidentified => return None,
    };

    let mut parts: Vec<&str> = Vec::with_capacity(5);
    if modifiers.control() {
        parts.push("Ctrl");
    }
    if modifiers.alt() {
        parts.push("Alt");
    }
    if modifiers.shift() {
        parts.push("Shift");
    }
    if modifiers.logo() {
        parts.push("Cmd");
    }
    parts.push(&key_token);
    Some(parts.join("+"))
}

/// Swap the current hotkey for a new one. Returns `Ok` on success; reverts on
/// failure.
pub fn update_hotkey(new_shortcut: &str) -> Result<(), HotkeyError> {
    let manager = HOTKEY_MANAGER.get().ok_or(HotkeyError::NotInitialized)?;
    let new = parse_hotkey(new_shortcut)?;
    let mut guard = CURRENT_HOTKEY.lock().unwrap();
    if let Some(old) = *guard {
        let _ = manager.unregister(old);
    }
    match manager.register(new) {
        Ok(_) => {
            *guard = Some(new);
            Ok(())
        }
        Err(e) => {
            // Try to restore the old binding.
            if let Some(old) = *guard {
                let _ = manager.register(old);
            }
            Err(HotkeyError::Register(e.to_string()))
        }
    }
}
