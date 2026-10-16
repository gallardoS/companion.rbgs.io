//! Keep the desktop process alive when its main window is closed.
use tauri::{
    Manager, WindowEvent,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

pub(crate) fn setup(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| std::io::Error::other("Main window unavailable"))?;
    let icon = app
        .default_window_icon()
        .ok_or_else(|| std::io::Error::other("Application icon unavailable"))?
        .clone();
    let open = MenuItem::with_id(
        app,
        "tray-open",
        "Open rbgs.io companion",
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let exit = MenuItem::with_id(app, "tray-exit", "Exit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &separator, &exit])?;

    TrayIconBuilder::with_id("companion")
        .icon(icon)
        .tooltip("rbgs.io companion")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-open" => crate::presence::show(app),
            "tray-exit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                crate::presence::show(tray.app_handle());
            }
        })
        .build(app)?;

    // Register only after the tray exists. Explicit Exit bypasses window close.
    let close_window = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            match close_window.hide() {
                Ok(()) => api.prevent_close(),
                Err(error) => eprintln!("Could not hide the companion window: {error}"),
            }
        }
    });
    Ok(())
}
