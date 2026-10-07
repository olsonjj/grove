use std::time::Instant;

use async_channel::Sender;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};

use crate::PlatformEvent;

pub fn install(tx: Sender<PlatformEvent>) -> global_hotkey::Result<GlobalHotKeyManager> {
    let switcher = HotKey::new(Some(Modifiers::ALT | Modifiers::SUPER), Code::KeyK);
    let main = HotKey::new(Some(Modifiers::ALT | Modifiers::SUPER), Code::KeyG);
    let (switcher_id, main_id) = (switcher.id(), main.id());

    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        let at = Instant::now();
        if event.state() != HotKeyState::Pressed {
            return;
        }
        let msg = if event.id() == switcher_id {
            PlatformEvent::ToggleSwitcher { at }
        } else if event.id() == main_id {
            PlatformEvent::ShowMain
        } else {
            return;
        };
        let _ = tx.try_send(msg);
    }));

    let manager = GlobalHotKeyManager::new()?;
    manager.register_all(&[switcher, main])?;
    eprintln!("[grove] hotkeys registered: alt+cmd+K, alt+cmd+G");
    Ok(manager)
}
