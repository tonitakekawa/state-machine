mod app;
mod fsm;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("状態機械")
            .with_inner_size([1024.0, 768.0]),
        ..Default::default()
    };
    eframe::run_native(
        "状態機械",
        options,
        Box::new(|cc| Ok(Box::new(app::FsmApp::new(cc)))),
    )
}
