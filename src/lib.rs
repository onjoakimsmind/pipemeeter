pub mod audio;
#[cfg(feature = "ui-preview")]
pub mod preview;
#[cfg(feature = "desktop-ui")]
pub mod ui;

pub fn run() -> Result<(), String> {
    #[cfg(feature = "desktop-ui")]
    {
        configure_desktop_linux_wayland_workaround();
        use std::sync::Arc;

        let bridge = Arc::new(audio::EngineBridge::spawn()?);
        ui::launch(bridge)
    }

    #[cfg(all(not(feature = "desktop-ui"), feature = "ui-preview"))]
    {
        preview::write_preview()?;
        Ok(())
    }

    #[cfg(all(not(feature = "desktop-ui"), not(feature = "ui-preview")))]
    {
        Err(
            "no runnable UI is enabled; use `cargo run --features ui-preview` for an HTML preview or `cargo run --features desktop-ui` for the native window".to_string(),
        )
    }
}

#[cfg(feature = "desktop-ui")]
fn configure_desktop_linux_wayland_workaround() {
    #[cfg(target_os = "linux")]
    {
        if !is_wayland_session(
            std::env::var("WAYLAND_DISPLAY").ok().as_deref(),
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        ) {
            return;
        }

        if env_var_is_unset_or_empty("WEBKIT_DISABLE_DMABUF_RENDERER") {
            // SAFETY: This runs before the desktop runtime spins up worker threads or initializes
            // GTK/WebKit. On affected Wayland systems, WebKitGTK can otherwise abort the
            // connection with a protocol error while setting up DMA-BUF rendering.
            unsafe {
                std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            }
        }
    }
}

#[cfg(all(feature = "desktop-ui", not(target_os = "linux")))]
fn configure_desktop_linux_wayland_workaround() {}

#[cfg(feature = "desktop-ui")]
fn env_var_is_unset_or_empty(name: &str) -> bool {
    std::env::var(name).map_or(true, |value| value.trim().is_empty())
}

#[cfg(feature = "desktop-ui")]
fn is_wayland_session(wayland_display: Option<&str>, xdg_session_type: Option<&str>) -> bool {
    wayland_display.is_some_and(|value| !value.trim().is_empty())
        || xdg_session_type.is_some_and(|value| value.eq_ignore_ascii_case("wayland"))
}

#[cfg(all(test, feature = "desktop-ui"))]
mod tests {
    use super::*;

    #[test]
    fn detects_wayland_sessions() {
        assert!(is_wayland_session(Some("wayland-0"), None));
        assert!(is_wayland_session(None, Some("wayland")));
        assert!(is_wayland_session(None, Some("WAYLAND")));
    }

    #[test]
    fn ignores_non_wayland_sessions() {
        assert!(!is_wayland_session(None, None));
        assert!(!is_wayland_session(Some(""), None));
        assert!(!is_wayland_session(None, Some("x11")));
    }
}
