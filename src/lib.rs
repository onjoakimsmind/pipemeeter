pub mod audio;
#[cfg(feature = "ui-preview")]
pub mod preview;
#[cfg(feature = "desktop-ui")]
pub mod ui;

pub fn run() -> Result<(), String> {
    #[cfg(feature = "desktop-ui")]
    {
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
