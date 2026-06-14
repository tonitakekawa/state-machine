use egui::{Color32, Pos2, Sense, Stroke, Vec2, Align2};
use std::path::PathBuf;
use crate::fsm::{Action, Fsm, Runner, TimeUnit, Variable};

const STATE_RADIUS: f32 = 36.0;
const ARROW_SIZE: f32 = 10.0;

#[derive(Debug, Clone, PartialEq)]
enum Tool {
    Select,
    AddState,
    AddTransition,
}

#[derive(Debug, Clone, PartialEq)]
enum Selection {
    None,
    State(String),
    Transition(String),
}

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    Edit,
    Run,
}

pub struct FsmApp {
    fsm: Fsm,
    tool: Tool,
    selection: Selection,
    mode: Mode,
    runner: Option<Runner>,
    event_input: String,
    // transition drawing
    transition_from: Option<String>,
    drag_pos: Option<Pos2>,
    // file
    current_file: Option<PathBuf>,
    status_message: Option<String>,
    // run loop (60Hz 固定タイムステップ用アキュムレータ)
    run_accumulator: f32,
    // ラベル変更用の入力バッファと対象状態 id
    label_input: String,
    label_input_id: Option<String>,
}

fn setup_fonts(ctx: &egui::Context) {
    // 日本語グリフを含むフォントを探して読み込む
    let candidates = [
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
    ];

    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "jp".to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(bytes)),
            );
            // すべてのフォントファミリの先頭に日本語フォントを差し込む
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "jp".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("jp".to_owned());
            ctx.set_fonts(fonts);
            return;
        }
    }
}

impl FsmApp {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        setup_fonts(&cc.egui_ctx);
        Self {
            fsm: Fsm::new(),
            tool: Tool::Select,
            selection: Selection::None,
            mode: Mode::Edit,
            runner: None,
            event_input: String::new(),
            transition_from: None,
            drag_pos: None,
            current_file: None,
            status_message: None,
            run_accumulator: 0.0,
            label_input: String::new(),
            label_input_id: None,
        }
    }

    fn save_to_file(&mut self) {
        if let Some(ref path) = self.current_file.clone() {
            match self.fsm.to_toml() {
                Ok(content) => {
                    if std::fs::write(path, content).is_ok() {
                        self.status_message = Some(format!("保存: {}", path.display()));
                    } else {
                        self.status_message = Some("保存失敗".to_string());
                    }
                }
                Err(e) => self.status_message = Some(format!("シリアライズ失敗: {}", e)),
            }
        } else {
            self.save_as();
        }
    }

    fn save_as(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("TOML", &["toml"])
            .save_file()
        {
            self.current_file = Some(path);
            self.save_to_file();
        }
    }

    fn open_file(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("TOML", &["toml"])
            .pick_file()
        {
            match std::fs::read_to_string(&path) {
                Ok(content) => match Fsm::from_toml(&content) {
                    Ok(fsm) => {
                        self.fsm = fsm;
                        self.current_file = Some(path.clone());
                        self.selection = Selection::None;
                        self.status_message = Some(format!("開いた: {}", path.display()));
                    }
                    Err(e) => self.status_message = Some(format!("読み込み失敗: {}", e)),
                },
                Err(e) => self.status_message = Some(format!("ファイル読み込み失敗: {}", e)),
            }
        }
    }

    /// 状態を既定位置に追加し、選択する（状態追加ボタン用）。
    /// 重ならないよう、状態数に応じて少しずつずらして配置する。
    fn add_state_default(&mut self) {
        let n = self.fsm.states.len() as f32;
        let x = 140.0 + (n % 6.0) * 110.0;
        let y = 120.0 + (n / 6.0).floor() * 110.0;
        let id = self.fsm.add_state("状態".to_string(), x, y);
        self.selection = Selection::State(id);
    }

    /// 指定座標（キャンバスローカル）に状態を追加し、選択する（画面タッチ用）。
    fn add_state_at(&mut self, x: f32, y: f32) {
        let id = self.fsm.add_state("状態".to_string(), x, y);
        self.selection = Selection::State(id);
    }

    fn state_at(&self, pos: Pos2) -> Option<String> {
        self.fsm.states.iter().find(|s| {
            let sp = Pos2::new(s.x, s.y);
            sp.distance(pos) <= STATE_RADIUS
        }).map(|s| s.id.clone())
    }

    fn draw_canvas(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(
            ui.available_size(),
            Sense::click_and_drag(),
        );

        let origin = response.rect.min;
        let to_screen = |p: Pos2| p + origin.to_vec2();
        let from_screen = |p: Pos2| p - origin.to_vec2();

        // background
        painter.rect_filled(response.rect, 0.0, Color32::from_gray(240));

        // draw transitions
        let active_transition_id = self.runner.as_ref().and_then(|r| r.active_transition.clone());
        let transitions = self.fsm.transitions.clone();
        for t in &transitions {
            let Some(from_state) = self.fsm.state_by_id(&t.from) else { continue };
            let Some(to_state) = self.fsm.state_by_id(&t.to) else { continue };

            let from_pos = to_screen(Pos2::new(from_state.x, from_state.y));
            let to_pos = to_screen(Pos2::new(to_state.x, to_state.y));

            let selected = self.selection == Selection::Transition(t.id.clone());
            let active = active_transition_id.as_deref() == Some(t.id.as_str());
            let color = if active {
                Color32::from_rgb(60, 170, 60)
            } else if selected {
                Color32::BLUE
            } else {
                Color32::DARK_GRAY
            };
            // オート遷移（イベント名なし）はラベルを「(auto)」と表示
            let event_label = if t.event.trim().is_empty() { "(auto)" } else { t.event.as_str() };

            if t.from == t.to {
                // self-loop
                let offset = Vec2::new(0.0, -STATE_RADIUS * 2.2);
                let ctrl = from_pos + offset;
                let p1 = from_pos + Vec2::new(-12.0, -STATE_RADIUS);
                let p2 = from_pos + Vec2::new(12.0, -STATE_RADIUS);
                painter.line_segment([p1, ctrl], Stroke::new(1.5, color));
                painter.line_segment([ctrl, p2], Stroke::new(1.5, color));
                painter.text(ctrl, Align2::CENTER_CENTER, event_label, egui::FontId::proportional(12.0), color);
            } else {
                let dir = (to_pos - from_pos).normalized();
                let start = from_pos + dir * STATE_RADIUS;
                let end = to_pos - dir * STATE_RADIUS;

                draw_arrow(&painter, start, end, color);

                let mid = (start + end.to_vec2()) / 2.0;
                let perp = Vec2::new(-dir.y, dir.x) * 14.0;
                painter.text(mid + perp, Align2::CENTER_CENTER, event_label, egui::FontId::proportional(12.0), color);
            }
        }

        // draw in-progress transition
        if let (Some(from_id), Some(drag)) = (&self.transition_from, self.drag_pos) {
            if let Some(from_state) = self.fsm.state_by_id(from_id) {
                let from_pos = to_screen(Pos2::new(from_state.x, from_state.y));
                painter.line_segment([from_pos, drag], Stroke::new(1.5, Color32::GRAY));
            }
        }

        // draw states
        let states = self.fsm.states.clone();
        let current_state_id = self.runner.as_ref().and_then(|r| r.current_state.clone());

        for state in &states {
            let center = to_screen(Pos2::new(state.x, state.y));
            let selected = self.selection == Selection::State(state.id.clone());
            let is_current = current_state_id.as_deref() == Some(&state.id);

            let fill = if is_current {
                Color32::from_rgb(100, 200, 100)
            } else if selected {
                Color32::from_rgb(180, 210, 255)
            } else {
                Color32::WHITE
            };
            let stroke_color = if selected { Color32::BLUE } else { Color32::DARK_GRAY };
            let stroke_width = if state.initial || selected { 2.5 } else { 1.5 };

            painter.circle(center, STATE_RADIUS, fill, Stroke::new(stroke_width, stroke_color));

            if state.accepting {
                painter.circle_stroke(center, STATE_RADIUS - 4.0, Stroke::new(1.5, stroke_color));
            }

            if state.initial {
                let arrow_end = center + Vec2::new(-STATE_RADIUS, 0.0);
                let arrow_start = arrow_end + Vec2::new(-24.0, 0.0);
                draw_arrow(&painter, arrow_start, arrow_end, stroke_color);
            }

            painter.text(center, Align2::CENTER_CENTER, &state.label, egui::FontId::proportional(14.0), Color32::BLACK);
        }

        // handle pointer interaction (編集モードのみ)
        if let (Mode::Edit, Some(pointer_pos)) = (self.mode.clone(), response.interact_pointer_pos()) {
            let local = from_screen(pointer_pos);

            match self.tool {
                Tool::AddState => {
                    // 空いている場所をタッチしたら、その位置に状態を追加。
                    // 既存の状態の上をタッチした場合は選択のみ（誤って重ねないように）。
                    if response.clicked() {
                        if let Some(id) = self.state_at(local) {
                            self.selection = Selection::State(id);
                        } else {
                            self.add_state_at(local.x, local.y);
                        }
                    }
                }
                Tool::AddTransition => {
                    if response.drag_started() {
                        self.transition_from = self.state_at(local);
                    }
                    if response.dragged() {
                        self.drag_pos = Some(pointer_pos);
                    }
                    if response.drag_stopped() {
                        if let Some(ref from_id) = self.transition_from.clone() {
                            if let Some(to_id) = self.state_at(local) {
                                self.fsm.add_transition(from_id.clone(), to_id, "event".to_string());
                            }
                        }
                        self.transition_from = None;
                        self.drag_pos = None;
                    }
                }
                Tool::Select => {
                    if response.clicked() {
                        // check states first
                        if let Some(id) = self.state_at(local) {
                            self.selection = Selection::State(id);
                        } else {
                            // check transitions (click near midpoint)
                            let mut found = None;
                            'outer: for t in &self.fsm.transitions {
                                let Some(fs) = self.fsm.state_by_id(&t.from) else { continue };
                                let Some(ts) = self.fsm.state_by_id(&t.to) else { continue };
                                let from_pos = Pos2::new(fs.x, fs.y);
                                let to_pos = Pos2::new(ts.x, ts.y);
                                let mid = (from_pos.to_vec2() + to_pos.to_vec2()) / 2.0;
                                if local.distance(Pos2::new(mid.x, mid.y)) < 16.0 {
                                    found = Some(t.id.clone());
                                    break 'outer;
                                }
                            }
                            self.selection = found.map(Selection::Transition).unwrap_or(Selection::None);
                        }
                    }
                    // drag states
                    if response.dragged() {
                        if let Selection::State(ref id) = self.selection.clone() {
                            if let Some(s) = self.fsm.state_by_id_mut(id) {
                                s.x += response.drag_delta().x;
                                s.y += response.drag_delta().y;
                            }
                        }
                    }
                }
            }
        }

        // delete key (編集モードのみ／テキスト入力中は無効)
        // ラベル等の編集中に Backspace で状態が消えるのを防ぐ
        let editing_text = ui.ctx().wants_keyboard_input();
        if self.mode == Mode::Edit
            && !editing_text
            && ui.input(|i| i.key_pressed(egui::Key::Backspace) || i.key_pressed(egui::Key::Delete))
        {
            match self.selection.clone() {
                Selection::State(id) => {
                    self.fsm.remove_state(&id);
                    self.selection = Selection::None;
                }
                Selection::Transition(id) => {
                    self.fsm.remove_transition(&id);
                    self.selection = Selection::None;
                }
                Selection::None => {}
            }
        }
    }

    fn draw_properties(&mut self, ui: &mut egui::Ui) {
        match self.selection.clone() {
            Selection::State(id) => {
                ui.heading("状態プロパティ");
                ui.separator();

                // 選択が変わったら入力欄を現在のラベルで初期化
                if self.label_input_id.as_deref() != Some(id.as_str()) {
                    self.label_input = self
                        .fsm
                        .state_by_id(&id)
                        .map(|s| s.label.clone())
                        .unwrap_or_default();
                    self.label_input_id = Some(id.clone());
                }

                ui.horizontal(|ui| {
                    ui.label("ラベル:");
                    let resp = ui.text_edit_singleline(&mut self.label_input);
                    let apply = ui.button("ラベル変更").clicked()
                        || (resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                    if apply {
                        if let Some(s) = self.fsm.state_by_id_mut(&id) {
                            s.label = self.label_input.clone();
                        }
                    }
                });

                if let Some(state) = self.fsm.state_by_id_mut(&id) {
                    ui.checkbox(&mut state.initial, "初期状態");
                    ui.checkbox(&mut state.accepting, "受理状態");
                }

                if ui.button("削除").clicked() {
                    self.fsm.remove_state(&id);
                    self.selection = Selection::None;
                }
            }
            Selection::Transition(id) => {
                ui.heading("遷移プロパティ");
                ui.separator();

                let t = self.fsm.transitions.iter_mut().find(|t| t.id == id);
                if let Some(t) = t {
                    ui.horizontal(|ui| {
                        ui.label("イベント:");
                        ui.text_edit_singleline(&mut t.event);
                    });
                    ui.label(
                        egui::RichText::new("空欄なら状態に入ると自動発火（オート遷移）")
                            .small()
                            .color(Color32::DARK_GRAY),
                    );

                    ui.separator();
                    ui.label("アクション（上から順に実行）:");

                    let mut to_remove: Option<usize> = None;
                    let mut move_up: Option<usize> = None;
                    for (i, action) in t.actions.iter_mut().enumerate() {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(format!("{}.", i + 1));
                                match action {
                                    Action::Wait { duration, unit } => {
                                        ui.label("待機");
                                        ui.add(
                                            egui::DragValue::new(duration)
                                                .range(0.0..=600000.0)
                                                .speed(1.0),
                                        );
                                        egui::ComboBox::from_id_salt(("unit", i))
                                            .selected_text(unit.label())
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(unit, TimeUnit::Ms, "ミリ秒");
                                                ui.selectable_value(unit, TimeUnit::Sec, "秒");
                                            });
                                    }
                                    Action::Message(msg) => {
                                        ui.label("メッセージ");
                                        ui.text_edit_singleline(msg);
                                    }
                                    Action::SetVar { name, value } => {
                                        ui.label("変数");
                                        ui.add(
                                            egui::TextEdit::singleline(name).desired_width(60.0),
                                        );
                                        ui.label("=");
                                        ui.add(
                                            egui::TextEdit::singleline(value).desired_width(80.0),
                                        );
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                if ui.small_button("⬆ 上へ").clicked() {
                                    move_up = Some(i);
                                }
                                if ui.small_button("削除").clicked() {
                                    to_remove = Some(i);
                                }
                            });
                        });
                    }
                    if let Some(i) = move_up {
                        if i > 0 {
                            t.actions.swap(i - 1, i);
                        }
                    }
                    if let Some(i) = to_remove {
                        t.actions.remove(i);
                    }

                    ui.horizontal(|ui| {
                        if ui.button("＋ 待機").clicked() {
                            t.actions.push(Action::Wait { duration: 1.0, unit: TimeUnit::Sec });
                        }
                        if ui.button("＋ メッセージ").clicked() {
                            t.actions.push(Action::Message(String::new()));
                        }
                        if ui.button("＋ 変数設定").clicked() {
                            t.actions.push(Action::SetVar {
                                name: String::new(),
                                value: String::new(),
                            });
                        }
                    });
                }

                if ui.button("削除").clicked() {
                    self.fsm.remove_transition(&id);
                    self.selection = Selection::None;
                }
            }
            Selection::None => {
                ui.label("状態または遷移を選択してください");
            }
        }
    }

    fn draw_context(&mut self, ui: &mut egui::Ui) {
        ui.heading("コンテクスト");
        ui.separator();

        match self.mode {
            Mode::Edit => {
                ui.label("変数（初期値）:");
                let mut to_remove: Option<usize> = None;
                for (i, v) in self.fsm.variables.iter_mut().enumerate() {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label("名前");
                            ui.add(egui::TextEdit::singleline(&mut v.name).desired_width(90.0));
                        });
                        ui.horizontal(|ui| {
                            ui.label("値　");
                            ui.add(egui::TextEdit::singleline(&mut v.value).desired_width(90.0));
                        });
                        if ui.small_button("削除").clicked() {
                            to_remove = Some(i);
                        }
                    });
                }
                if let Some(i) = to_remove {
                    self.fsm.variables.remove(i);
                }
                if ui.button("＋ 変数").clicked() {
                    let n = self.fsm.variables.len() + 1;
                    self.fsm.variables.push(Variable {
                        name: format!("var{}", n),
                        value: String::new(),
                    });
                }

                ui.separator();
                ui.label(
                    egui::RichText::new("メッセージや変数設定で {名前} と書くと\n実行時に値へ置換されます")
                        .small()
                        .color(Color32::DARK_GRAY),
                );
            }
            Mode::Run => {
                ui.label("変数（実行中）:");
                if let Some(runner) = self.runner.as_mut() {
                    if runner.vars.is_empty() {
                        ui.label(egui::RichText::new("（変数なし）").color(Color32::GRAY));
                    } else {
                        let mut keys: Vec<String> = runner.vars.keys().cloned().collect();
                        keys.sort();
                        for k in keys {
                            ui.horizontal(|ui| {
                                ui.label(format!("{} =", k));
                                if let Some(val) = runner.vars.get_mut(&k) {
                                    ui.add(egui::TextEdit::singleline(val).desired_width(90.0));
                                }
                            });
                        }
                    }
                }
            }
        }
    }

    fn draw_run_panel(&mut self, ui: &mut egui::Ui) {
        if self.runner.is_none() {
            self.runner = Some(Runner::new(&self.fsm));
        }
        let runner = self.runner.as_mut().unwrap();

        let current_label = runner.current_state
            .as_deref()
            .and_then(|id| self.fsm.state_by_id(id))
            .map(|s| s.label.as_str())
            .unwrap_or("なし");

        ui.heading("実行 (60Hz)");
        ui.separator();
        ui.label(format!("現在の状態: {}", current_label));

        // メッセージ表示
        ui.separator();
        ui.label("メッセージ:");
        if let Some(ref msg) = runner.message {
            ui.label(egui::RichText::new(msg).size(20.0).strong());
        } else {
            ui.label(egui::RichText::new("（なし）").color(Color32::GRAY));
        }
        ui.separator();

        // 名前付きイベントの手動発火（分岐用）
        ui.horizontal(|ui| {
            ui.label("イベント:");
            ui.text_edit_singleline(&mut self.event_input);
            if ui.button("送信").clicked() {
                let event = self.event_input.clone();
                if !event.is_empty() {
                    runner.trigger(&self.fsm, &event);
                    self.event_input.clear();
                }
            }
        });

        if ui.button("リセット").clicked() {
            runner.reset(&self.fsm);
        }

        ui.separator();
        ui.label("ログ:");
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            for entry in runner.log.iter().rev() {
                ui.label(entry);
            }
        });
    }
}

impl eframe::App for FsmApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 実行モードでは状態機械を 60Hz 固定タイムステップで駆動する
        if self.mode == Mode::Run {
            if self.runner.is_none() {
                self.runner = Some(Runner::new(&self.fsm));
            }
            let dt = ctx.input(|i| i.stable_dt).min(0.1);
            let step = 1.0 / 60.0;
            let mut acc = self.run_accumulator + dt;
            if let Some(runner) = self.runner.as_mut() {
                while acc >= step {
                    runner.tick(&self.fsm, step);
                    acc -= step;
                }
            }
            self.run_accumulator = acc;
            ctx.request_repaint();
        } else {
            self.run_accumulator = 0.0;
        }

        // keyboard shortcuts
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
            self.save_to_file();
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::O)) {
            self.open_file();
        }

        // top menu bar
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("ファイル", |ui| {
                    if ui.button("開く (⌘O)").clicked() {
                        self.open_file();
                        ui.close_menu();
                    }
                    if ui.button("保存 (⌘S)").clicked() {
                        self.save_to_file();
                        ui.close_menu();
                    }
                    if ui.button("名前を付けて保存").clicked() {
                        self.save_as();
                        ui.close_menu();
                    }
                });

                ui.separator();

                ui.selectable_value(&mut self.mode, Mode::Edit, "編集");
                ui.selectable_value(&mut self.mode, Mode::Run, "実行");

                if let Some(ref msg) = self.status_message.clone() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(msg).small().color(Color32::DARK_GRAY));
                    });
                }
            });
        });

        // toolbar (edit mode only)
        if self.mode == Mode::Edit {
            egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tool, Tool::Select, "選択");
                    ui.selectable_value(&mut self.tool, Tool::AddState, "状態追加(タッチ)");
                    ui.selectable_value(&mut self.tool, Tool::AddTransition, "遷移追加");
                    ui.separator();
                    if ui.button("状態追加").clicked() {
                        self.add_state_default();
                    }
                });
            });
        }

        // left context panel (画面幅の 1/4)
        let context_width = (ctx.screen_rect().width() * 0.25).max(180.0);
        egui::SidePanel::left("context")
            .exact_width(context_width)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.draw_context(ui);
                });
            });

        // right panel
        egui::SidePanel::right("properties").min_width(220.0).show(ctx, |ui| {
            match self.mode {
                Mode::Edit => self.draw_properties(ui),
                Mode::Run => self.draw_run_panel(ui),
            }
        });

        // canvas
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_canvas(ui);
        });
    }
}

fn draw_arrow(painter: &egui::Painter, start: Pos2, end: Pos2, color: Color32) {
    painter.line_segment([start, end], Stroke::new(1.5, color));

    let dir = (end - start).normalized();
    let perp = Vec2::new(-dir.y, dir.x);
    let tip1 = end - dir * ARROW_SIZE + perp * (ARROW_SIZE * 0.5);
    let tip2 = end - dir * ARROW_SIZE - perp * (ARROW_SIZE * 0.5);
    painter.line_segment([end, tip1], Stroke::new(1.5, color));
    painter.line_segment([end, tip2], Stroke::new(1.5, color));
}
