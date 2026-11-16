#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use multiia::app;

fn icon() -> eframe::egui::IconData {
    eframe::egui::IconData {
        rgba: include_bytes!("../assets/icon.rgba").to_vec(),
        width: 128,
        height: 128,
    }
}

fn options(renderer: eframe::Renderer) -> eframe::NativeOptions {
    eframe::NativeOptions {
        renderer,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(format!(
                "{} di Lorenzo D'Apolito v{}",
                app::APP_TITLE,
                app::APP_VERSION
            ))
            .with_inner_size([1250.0, 780.0])
            .with_min_inner_size([760.0, 480.0])
            .with_icon(icon()),
        ..Default::default()
    }
}

fn report_fatal(text: &str) {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let _ = std::fs::write(dir.join("MultiIA-errore.txt"), text);
        }
    }
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("MultiIA non riesce ad avviarsi")
        .set_description(text)
        .show();
}

fn run() -> eframe::Result {
    let mut last = Ok(());
    for renderer in [eframe::Renderer::Wgpu, eframe::Renderer::Glow] {
        last = eframe::run_native(
            app::APP_TITLE,
            options(renderer),
            Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
        );
        if last.is_ok() {
            break;
        }
    }
    last
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        report_fatal(&format!(
            "MultiIA v{}\nErrore imprevisto.\n\n{info}",
            app::APP_VERSION
        ));
    }));
    if let Err(e) = run() {
        report_fatal(&format!(
            "MultiIA v{}\nLa finestra non si apre: serve DirectX/Vulkan o OpenGL. Installa i driver video dal sito del produttore.\n\n{e}",
            app::APP_VERSION
        ));
    }
}
