use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 待機時間の単位（仕様: ms, sec で指定）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TimeUnit {
    Ms,
    Sec,
}

impl TimeUnit {
    pub fn label(&self) -> &'static str {
        match self {
            TimeUnit::Ms => "ミリ秒",
            TimeUnit::Sec => "秒",
        }
    }
}

/// 状態（遷移）に紐づくアクション。
/// 仕様: 待機 / メッセージ / イベント(状態遷移)。
/// 「イベント(状態遷移)」は遷移そのもの（from→to）が担うため、
/// アクション列としては待機・メッセージを順次実行する。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Action {
    Wait { duration: f32, unit: TimeUnit },
    Message(String),
    /// 変数への代入（value 内の {名前} は実行時に展開される）
    SetVar { name: String, value: String },
}

impl Action {
    /// 待機アクションの秒数（待機以外は 0）
    pub fn wait_secs(&self) -> f32 {
        match self {
            Action::Wait { duration, unit } => match unit {
                TimeUnit::Ms => duration / 1000.0,
                TimeUnit::Sec => *duration,
            },
            _ => 0.0,
        }
    }
}

/// コンテクストで扱う変数（名前と値）。初期値として保存される。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub id: String,
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub initial: bool,
    pub accepting: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    pub id: String,
    pub from: String,
    pub to: String,
    /// 遷移を起こすイベント名。空文字ならオート遷移（状態に入ると自動発火）。
    pub event: String,
    /// 遷移発火時に上から順に実行するアクション列。
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Fsm {
    pub states: Vec<State>,
    pub transitions: Vec<Transition>,
    /// コンテクスト変数（初期値）。
    #[serde(default)]
    pub variables: Vec<Variable>,
}

impl Fsm {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_state(&mut self, label: String, x: f32, y: f32) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let initial = self.states.is_empty();
        self.states.push(State { id: id.clone(), label, x, y, initial, accepting: false });
        id
    }

    pub fn add_transition(&mut self, from: String, to: String, event: String) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.transitions.push(Transition { id: id.clone(), from, to, event, actions: Vec::new() });
        id
    }

    pub fn remove_state(&mut self, id: &str) {
        self.states.retain(|s| s.id != id);
        self.transitions.retain(|t| t.from != id && t.to != id);
    }

    pub fn remove_transition(&mut self, id: &str) {
        self.transitions.retain(|t| t.id != id);
    }

    pub fn state_by_id(&self, id: &str) -> Option<&State> {
        self.states.iter().find(|s| s.id == id)
    }

    pub fn state_by_id_mut(&mut self, id: &str) -> Option<&mut State> {
        self.states.iter_mut().find(|s| s.id == id)
    }

    pub fn transitions_from(&self, state_id: &str) -> Vec<&Transition> {
        self.transitions.iter().filter(|t| t.from == state_id).collect()
    }

    pub fn initial_state(&self) -> Option<&State> {
        self.states.iter().find(|s| s.initial)
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
}

/// 実行中に再生中の遷移の進捗。
#[derive(Debug)]
struct Playback {
    transition_id: String,
    action_index: usize,
    wait_remaining: f32, // 秒
    started_wait: bool,
}

/// 状態機械を 60Hz で駆動するランナー。
#[derive(Debug)]
pub struct Runner {
    pub current_state: Option<String>,
    /// 現在再生中の遷移（描画ハイライト用）
    pub active_transition: Option<String>,
    /// 現在表示中のメッセージ
    pub message: Option<String>,
    pub log: Vec<String>,
    /// 実行中の変数（名前→値）
    pub vars: HashMap<String, String>,
    play: Option<Playback>,
}

impl Runner {
    pub fn new(fsm: &Fsm) -> Self {
        Self {
            current_state: fsm.initial_state().map(|s| s.id.clone()),
            active_transition: None,
            message: None,
            log: Vec::new(),
            vars: Self::init_vars(fsm),
            play: None,
        }
    }

    pub fn reset(&mut self, fsm: &Fsm) {
        self.current_state = fsm.initial_state().map(|s| s.id.clone());
        self.active_transition = None;
        self.message = None;
        self.vars = Self::init_vars(fsm);
        self.play = None;
        self.log.clear();
    }

    fn init_vars(fsm: &Fsm) -> HashMap<String, String> {
        fsm.variables
            .iter()
            .map(|v| (v.name.clone(), v.value.clone()))
            .collect()
    }

    /// テキスト中の {変数名} を現在の値に置換する。
    fn interpolate(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (k, v) in &self.vars {
            out = out.replace(&format!("{{{}}}", k), v);
        }
        out
    }

    /// 名前付きイベントを発火して対応する遷移を開始する。
    pub fn trigger(&mut self, fsm: &Fsm, event: &str) -> bool {
        if self.play.is_some() {
            self.log.push("実行中のため受け付けません".to_string());
            return false;
        }
        let Some(cur) = self.current_state.clone() else { return false };
        let id = match fsm.transitions_from(&cur).into_iter().find(|t| t.event == event) {
            Some(t) => t.id.clone(),
            None => {
                self.log.push(format!("イベント '{}' に対応する遷移なし", event));
                return false;
            }
        };
        self.log.push(format!("イベント '{}' 受信", event));
        self.start(id);
        true
    }

    /// 1 ステップ（dt 秒）進める。App 側から 60Hz 固定で呼ぶ。
    pub fn tick(&mut self, fsm: &Fsm, dt: f32) {
        if self.play.is_none() {
            self.try_start_auto(fsm);
        }
        let Some(mut play) = self.play.take() else { return };
        let Some(t) = fsm.transitions.iter().find(|t| t.id == play.transition_id).cloned() else {
            self.active_transition = None;
            return;
        };

        loop {
            // アクション列を消化し終えたら遷移先へ到達
            if play.action_index >= t.actions.len() {
                self.current_state = Some(t.to.clone());
                self.active_transition = None;
                self.message = None;
                let label = fsm.state_by_id(&t.to).map(|s| s.label.clone()).unwrap_or_default();
                self.log.push(format!("→ {}", label));
                return;
            }

            match &t.actions[play.action_index] {
                Action::Wait { .. } => {
                    if !play.started_wait {
                        play.started_wait = true;
                        play.wait_remaining = t.actions[play.action_index].wait_secs();
                    }
                    play.wait_remaining -= dt;
                    if play.wait_remaining > 0.0 {
                        // まだ待機中。状態を保持して次フレームへ。
                        self.play = Some(play);
                        return;
                    }
                    play.started_wait = false;
                    play.action_index += 1;
                }
                Action::Message(msg) => {
                    let rendered = self.interpolate(msg);
                    self.message = Some(rendered.clone());
                    self.log.push(format!("[メッセージ] {}", rendered));
                    play.action_index += 1;
                }
                Action::SetVar { name, value } => {
                    let rendered = self.interpolate(value);
                    self.vars.insert(name.clone(), rendered.clone());
                    self.log.push(format!("[変数] {} = {}", name, rendered));
                    play.action_index += 1;
                }
            }
        }
    }

    /// 現在状態から event 名が空（オート）の遷移があれば開始する。
    fn try_start_auto(&mut self, fsm: &Fsm) {
        let Some(cur) = self.current_state.clone() else { return };
        let id = fsm
            .transitions_from(&cur)
            .into_iter()
            .find(|t| t.event.trim().is_empty())
            .map(|t| t.id.clone());
        if let Some(id) = id {
            self.start(id);
        }
    }

    fn start(&mut self, transition_id: String) {
        self.active_transition = Some(transition_id.clone());
        self.play = Some(Playback {
            transition_id,
            action_index: 0,
            wait_remaining: 0.0,
            started_wait: false,
        });
    }
}
