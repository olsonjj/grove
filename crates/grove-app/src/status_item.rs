use async_channel::Sender;
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::{PlatformEvent, macos};

/// Status item frame in global logical points, origin at the primary screen's top-left.
#[derive(Clone, Copy, Debug)]
pub struct Anchor {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    /// The screen whose menu bar was clicked.
    pub screen: Option<macos::Screen>,
}

pub fn install(tx: Sender<PlatformEvent>) -> tray_icon::Result<TrayIcon> {
    TrayIconEvent::set_event_handler(Some(move |event| {
        if let TrayIconEvent::Click {
            rect,
            button: MouseButton::Left,
            button_state: MouseButtonState::Down,
            ..
        } = event
        {
            // The mouse is on the clicked menu bar; `rect` is in that screen's pixels.
            let screen = macos::screen_under_mouse();
            let scale = screen.map_or(2.0, |s| s.scale);
            let anchor = Anchor {
                x: (rect.position.x / scale) as f32,
                y: (rect.position.y / scale) as f32,
                width: (rect.size.width as f64 / scale) as f32,
                height: (rect.size.height as f64 / scale) as f32,
                screen,
            };
            let _ = tx.try_send(PlatformEvent::StatusItemClicked { anchor });
        }
    }));

    let tray = TrayIconBuilder::new()
        .with_id("grove")
        .with_tooltip("Grove")
        .with_icon_templated(icon())
        .build()?;
    eprintln!("[grove] status item installed");
    Ok(tray)
}

/// Placeholder glyph: a filled circle (canopy) on a stem.
fn icon() -> Icon {
    const N: u32 = 36;
    let mut rgba = vec![0u8; (N * N * 4) as usize];
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let canopy = (fx - 18.0).powi(2) + (fy - 14.0).powi(2) <= 11.0f32.powi(2);
            let stem = (16.0..20.0).contains(&fx) && (20.0..34.0).contains(&fy);
            if canopy || stem {
                let i = ((y * N + x) * 4) as usize;
                rgba[i + 3] = 255;
            }
        }
    }
    Icon::from_rgba(rgba, N, N).expect("valid icon")
}
