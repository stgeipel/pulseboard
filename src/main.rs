#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod dashboard;
#[cfg(target_os = "windows")]
mod windows;

use dashboard::Dashboard;
use gpui::{
    App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, prelude::*, px, size,
};

fn main() {
    #[cfg(target_os = "windows")]
    let _instance = match windows::acquire_instance() {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(error) => {
            windows::show_error(&format!(
                "Pulseboard konnte nicht gestartet werden. {error}"
            ));
            return;
        }
    };

    #[cfg(target_os = "windows")]
    windows::install_crash_log();

    Application::new().run(|cx: &mut App| {
        #[cfg(target_os = "windows")]
        let tray = windows::Tray::new();
        #[cfg(target_os = "windows")]
        let hidden = tray.is_ok() && std::env::args().any(|arg| arg == "--background");
        #[cfg(not(target_os = "windows"))]
        let hidden = false;

        let bounds = Bounds::centered(None, size(px(1100.0), px(750.0)), cx);
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(860.0), px(580.0))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Pulseboard".into()),
                    ..Default::default()
                }),
                show: !hidden,
                focus: !hidden,
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    Dashboard::new(
                        window,
                        cx,
                        #[cfg(target_os = "windows")]
                        tray,
                    )
                })
            },
        );
        if let Err(error) = window {
            #[cfg(target_os = "windows")]
            windows::show_error(&format!(
                "Das Fenster konnte nicht geöffnet werden. {error}"
            ));
            #[cfg(not(target_os = "windows"))]
            eprintln!("Das Fenster konnte nicht geöffnet werden. {error}");
            cx.quit();
        } else if !hidden {
            cx.activate(true);
        }
    });
}
