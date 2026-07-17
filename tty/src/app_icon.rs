//! App-icon glue: a window/taskbar icon (Linux/Windows) and the macOS Dock icon.
//!
//! macOS ignores *window* icons, and a bare `cargo run` binary isn't an `.app` bundle,
//! so the Dock icon must be set at runtime — and *after* launch (a call before the event
//! loop is reset by winit). [`ensure_dock_icon`] does that once, from `view`. The
//! packaged app reads `AppIcon.icns` from its bundle instead.

/// Set the macOS Dock icon from PNG bytes (`NSImage` decodes the PNG). No-op off macOS or
/// off the main thread.
#[cfg(target_os = "macos")]
pub fn set_dock_icon(png: &[u8]) {
    use objc2::AnyThread;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{MainThreadMarker, NSData};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(png);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: standard AppKit call; the image is retained by the application.
    unsafe { app.setApplicationIconImage(Some(&image)) };
}

/// No-op on non-macOS platforms (the Dock is macOS-only).
#[cfg(not(target_os = "macos"))]
pub fn set_dock_icon(_png: &[u8]) {}

/// Set the macOS Dock icon exactly once, on first call. Call from `view` (runs on the
/// main thread after launch, so it sticks). Cheap after the first call.
pub fn ensure_dock_icon(png: &'static [u8]) {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| set_dock_icon(png));
}

/// Decode PNG bytes into an `iced` window icon (Linux/Windows taskbar). `None` if it can't
/// be decoded. macOS ignores window icons — use [`set_dock_icon`] there.
pub fn window_icon(png: &[u8]) -> Option<iced::window::Icon> {
    let decoder = png::Decoder::new(png);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        _ => return None,
    };
    iced::window::icon::from_rgba(rgba, info.width, info.height).ok()
}
