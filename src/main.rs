mod app;
mod fsm;

fn main() -> eframe::Result
{
    let viewport = egui::ViewportBuilder::default();
    let vp_title  = viewport.with_title("状態機械");
    let sz       = vp_title.with_inner_size([1024.0, 768.0]);
    let options  = eframe::NativeOptions { viewport: sz, ..Default::default() };

    let e_frame = eframe::run_native(
        "状態機械",
        options,
        Box::new(|cc| Ok(Box::new(app::FsmApp::new(cc)))),
    );

    return e_frame;
}
