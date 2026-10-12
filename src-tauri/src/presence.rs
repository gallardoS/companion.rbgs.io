//! Native lifecycle: independent of browser/webview timers and minimized windows.
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;
pub(crate) fn allowed_link(url: &str) -> bool {
    url == "rbgs-companion://open" || url == "rbgs-companion://open/"
}
fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
pub(crate) fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Freeze the launch environment before any pairing, heartbeat or inbox work.
    crate::pairing::client().map_err(std::io::Error::other)?;
    #[cfg(all(debug_assertions, windows))]
    if app.deep_link().register_all().is_err() {
        eprintln!("Launch link registration unavailable; start the companion from Windows.");
    }
    let handle = app.handle().clone();
    app.deep_link().on_open_url(move |event| {
        if event.urls().iter().any(|u| allowed_link(u.as_str())) {
            show(&handle);
        }
    });
    if app
        .deep_link()
        .get_current()?
        .is_some_and(|urls| urls.iter().any(|u| allowed_link(u.as_str())))
    {
        show(app.handle());
    }
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let mut next = std::time::Instant::now();
        loop {
            // Pending approval exchange also continues when the webview is suspended.
            let _ =
                crate::pairing::advance_pairing(&handle.state::<crate::pairing::PairingState>())
                    .await;
            if std::time::Instant::now() >= next {
                let _ = crate::pairing::send_heartbeat().await;
                next = std::time::Instant::now() + std::time::Duration::from_secs(15);
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launch_links_cannot_supply_tokens_destinations_or_commands() {
        assert!(allowed_link("rbgs-companion://open"));
        for bad in [
            "rbgs-companion://connect",
            "rbgs-companion://open?token=x",
            "rbgs-companion://open#token=x",
            "https://rbgs.io",
            "rbgs-companion://evil/open",
        ] {
            assert!(!allowed_link(bad));
        }
    }
}
