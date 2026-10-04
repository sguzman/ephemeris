use ephemeris::EphemerisApp;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Ephemeris")
            .with_inner_size([1440.0, 920.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Ephemeris",
        options,
        Box::new(|_cc| {
            EphemerisApp::open()
                .map(|app| Box::new(app) as Box<dyn eframe::App>)
                .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })
        }),
    )?;

    Ok(())
}
