//! Keys and mouse for the Gonzo-style log/insights area. The key table is in
//! `insights_ui::keys`; this file applies each key to the app.

use std::time::Instant;

use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use regex::Regex;

use super::{contains, App, FocusArea};
use crate::events::{Action, Mode};
use crate::insights::severity::Severity;
use crate::insights_ui::cursor::{keep_visible, step};
use crate::insights_ui::detail::{clipboard_text, detail};
use crate::insights_ui::keys::{route, DoxKey, InsightsKey, KeyContext};
use crate::insights_ui::picker::{apply_columns, apply_severity, column_picker, severity_picker};
use crate::insights_ui::{Input, InputKind, Modal, Section};

/// Fields offered in the column picker: the most varied ones seen so far.
const PICKER_FIELDS: usize = 20;
/// Characters per Left/Right press in the log list.
const H_SCROLL_STEP: usize = 10;
/// Rows per mouse wheel notch.
const WHEEL_ROWS: isize = 3;

impl App {
    /// Keys go to the insights handling while the log area is focused.
    pub(super) fn insights_has_keyboard(&self) -> bool {
        matches!(self.focus, FocusArea::Detail) && matches!(self.mode, Mode::Normal)
    }

    pub(super) fn handle_insights_key(&mut self, key: KeyEvent) {
        let ctx = KeyContext {
            typing: self.insights.input.is_some(),
            modal: self.insights.modal.as_ref().map(Modal::kind),
        };
        let action = route(&key, &ctx);
        tracing::debug!(?key, ?action, "insights key");
        self.apply_insights_key(action);
    }

    /// Wheel and clicks on the side panel, and anything while a popup is open.
    /// Returns true when handled here.
    pub(super) fn handle_insights_mouse(&mut self, m: &MouseEvent) -> bool {
        let wheel = match m.kind {
            MouseEventKind::ScrollUp => -WHEEL_ROWS,
            MouseEventKind::ScrollDown => WHEEL_ROWS,
            _ => 0,
        };
        if self.insights.modal.is_some() {
            if wheel != 0 {
                self.insights_move(wheel);
            }
            return true;
        }
        if contains(self.last_log_inner, m.column, m.row) {
            self.insights.section = Section::Logs;
            return false;
        }
        let areas = self.insights.box_areas;
        let Some(section) = areas
            .iter()
            .position(|r| contains(*r, m.column, m.row))
            .and_then(Section::from_box)
        else {
            return false;
        };
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.focus = FocusArea::Detail;
                self.insights.section = section;
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                self.insights.section = section;
                self.insights_move(wheel);
            }
            _ => return false,
        }
        true
    }

    fn apply_insights_key(&mut self, key: InsightsKey) {
        match key {
            InsightsKey::NextSection => self.insights.section = self.insights.section.next(),
            InsightsKey::PrevSection => self.insights.section = self.insights.section.prev(),
            InsightsKey::Up => self.insights_move(-1),
            InsightsKey::Down => self.insights_move(1),
            InsightsKey::PageUp => self.insights_move(-(self.page_rows() as isize)),
            InsightsKey::PageDown => self.insights_move(self.page_rows() as isize),
            InsightsKey::Home => self.insights_home(),
            InsightsKey::End => self.insights_end(),
            InsightsKey::Left => {
                self.insights.h_scroll = self.insights.h_scroll.saturating_sub(H_SCROLL_STEP)
            }
            InsightsKey::Right => self.insights.h_scroll += H_SCROLL_STEP,
            InsightsKey::Open => self.insights_open(),
            InsightsKey::TogglePause => self.toggle_insights_pause(),
            InsightsKey::BeginFilter => self.begin_input(InputKind::Filter),
            InsightsKey::BeginSearch => self.begin_input(InputKind::Search),
            InsightsKey::SeverityFilter => {
                let picker = severity_picker(&self.insights.filter.hidden);
                self.open_modal(Modal::SeverityFilter(picker));
            }
            InsightsKey::Columns => {
                let fields: Vec<String> = self
                    .insights
                    .snapshot
                    .attributes
                    .iter()
                    .take(PICKER_FIELDS)
                    .map(|a| a.key.clone())
                    .collect();
                let picker = column_picker(&self.insights.columns, &fields);
                self.open_modal(Modal::Columns(picker));
            }
            InsightsKey::Fullscreen => self.open_modal(Modal::Fullscreen),
            InsightsKey::Stats => self.open_modal(Modal::Stats),
            InsightsKey::Reset => {
                let now = Instant::now();
                self.insights.engine.reset(now);
                self.insights.counts_history.clear();
                self.insights.refresh_now(now);
                self.set_toast("insights reset");
            }
            InsightsKey::SlowerRefresh => {
                self.insights.slower();
                self.toast_interval();
            }
            InsightsKey::FasterRefresh => {
                self.insights.faster();
                self.toast_interval();
            }
            InsightsKey::CloseModal => self.insights.modal = None,
            InsightsKey::ToggleItem => {
                if let Some(Modal::SeverityFilter(p) | Modal::Columns(p)) =
                    self.insights.modal.as_mut()
                {
                    p.toggle();
                }
            }
            InsightsKey::Apply => self.apply_picker(),
            InsightsKey::CopyMessage => self.copy_detail(false),
            InsightsKey::CopyEntry => self.copy_detail(true),
            InsightsKey::InputChar(c) => self.edit_input(|text| text.push(c)),
            InsightsKey::InputBackspace => self.edit_input(|text| {
                text.pop();
            }),
            InsightsKey::InputCommit => self.insights.input = None,
            InsightsKey::InputCancel => self.cancel_input(),
            InsightsKey::Dox(k) => self.apply_dox_key(k),
            InsightsKey::Ignore => {}
        }
    }

    fn apply_dox_key(&mut self, key: DoxKey) {
        match key {
            DoxKey::Quit => self.should_quit = true,
            DoxKey::RestartStream => self.apply_action(Action::RestartLogStream),
            DoxKey::GrowPanels => self.apply_action(Action::GrowPanels),
            DoxKey::ShrinkPanels => self.apply_action(Action::ShrinkPanels),
            DoxKey::ResetPanels => self.apply_action(Action::ResetPanels),
            DoxKey::Help => self.mode = Mode::Help,
            DoxKey::ToggleMouse => self.apply_action(Action::ToggleMouseCapture),
            DoxKey::Visual => self.apply_action(Action::EnterVisualMode),
            DoxKey::Yank => self.apply_action(Action::YankSelection),
            DoxKey::Unfocus => {
                self.insights.log_cursor = None;
                self.apply_action(Action::UnfocusDetail);
            }
        }
    }

    fn page_rows(&self) -> usize {
        self.last_log_height.max(1)
    }

    /// Up/Down for whatever has the focus: a picker, a popup's scroll, the
    /// log cursor, or a side-panel box.
    fn insights_move(&mut self, delta: isize) {
        if let Some(Modal::SeverityFilter(p) | Modal::Columns(p)) = self.insights.modal.as_mut() {
            p.move_by(delta);
            return;
        }
        match (&self.insights.modal, self.insights.section) {
            (Some(Modal::Fullscreen), _) | (None, Section::Logs) => self.move_log_cursor(delta),
            (Some(_), _) => {
                let scroll = self.insights.modal_scroll as isize + delta;
                self.insights.modal_scroll = scroll.max(0) as usize;
            }
            (None, section) => {
                let Some(idx) = section.box_index() else {
                    return;
                };
                let len = self.box_len(section);
                let current = self.insights.selected[idx];
                self.insights.selected[idx] = step(Some(current), 0, delta, len).unwrap_or(0);
            }
        }
    }

    fn box_len(&self, section: Section) -> usize {
        let snap = &self.insights.snapshot;
        match section {
            Section::Words => snap.words.len(),
            Section::Attributes => snap.attributes.len(),
            Section::Patterns => snap.patterns.len(),
            Section::Counts => Severity::ALL.len(),
            Section::Logs => self.log_view().len(),
        }
    }

    /// First log row on screen, as `ui::logs::draw` computes it.
    fn first_visible_log(&self) -> usize {
        let len = self.log_view().len();
        if self.logs_follow {
            len.saturating_sub(self.page_rows())
        } else {
            self.logs_scroll.min(len.saturating_sub(1))
        }
    }

    fn move_log_cursor(&mut self, delta: isize) {
        let len = self.log_view().len();
        let height = self.page_rows();
        let first = self.first_visible_log();
        let last_visible = (first + height).min(len).saturating_sub(1);
        let Some(cursor) = step(self.insights.log_cursor, last_visible, delta, len) else {
            return;
        };
        self.insights.log_cursor = Some(cursor);
        self.logs_follow = false;
        self.logs_scroll = keep_visible(cursor, first, height);
    }

    fn insights_home(&mut self) {
        if let Some(Modal::SeverityFilter(p) | Modal::Columns(p)) = self.insights.modal.as_mut() {
            p.cursor = 0;
            return;
        }
        match (&self.insights.modal, self.insights.section) {
            (Some(Modal::Fullscreen), _) | (None, Section::Logs) => {
                self.insights.log_cursor = (!self.log_view().is_empty()).then_some(0);
                self.logs_follow = false;
                self.logs_scroll = 0;
            }
            (Some(_), _) => self.insights.modal_scroll = 0,
            (None, section) => {
                if let Some(idx) = section.box_index() {
                    self.insights.selected[idx] = 0;
                }
            }
        }
    }

    /// End: back to the newest logs with live follow (Gonzo's resume).
    fn insights_end(&mut self) {
        if let Some(Modal::SeverityFilter(p) | Modal::Columns(p)) = self.insights.modal.as_mut() {
            p.cursor = p.items.len().saturating_sub(1);
            return;
        }
        match (&self.insights.modal, self.insights.section) {
            (Some(Modal::Fullscreen), _) | (None, Section::Logs) => {
                self.insights.log_cursor = None;
                self.logs_follow = true;
                self.logs_scroll = self.log_view().len();
            }
            (Some(_), _) => self.insights.modal_scroll = usize::MAX / 2,
            (None, section) => {
                if let Some(idx) = section.box_index() {
                    self.insights.selected[idx] = self.box_len(section).saturating_sub(1);
                }
            }
        }
    }

    /// Enter: log detail, highlight a word, a field's values, all patterns,
    /// or the counts analysis — as in Gonzo.
    fn insights_open(&mut self) {
        match (&self.insights.modal, self.insights.section) {
            (Some(Modal::Fullscreen), _) | (None, Section::Logs) => self.open_log_detail(),
            (Some(_), _) => {}
            (None, Section::Words) => {
                let word = self
                    .insights
                    .snapshot
                    .words
                    .get(self.insights.selected[0])
                    .map(|(w, _)| w.clone());
                if let Some(word) = word {
                    let same = self.insights.search.as_deref() == Some(word.as_str());
                    self.insights.search = (!same).then_some(word);
                }
            }
            (None, Section::Attributes) => {
                let key = self
                    .insights
                    .snapshot
                    .attributes
                    .get(self.insights.selected[1])
                    .map(|a| a.key.clone());
                if let Some(key) = key {
                    self.open_modal(Modal::AttributeValues { key });
                }
            }
            (None, Section::Patterns) => self.open_modal(Modal::Patterns),
            (None, Section::Counts) => self.open_modal(Modal::Counts),
        }
    }

    fn open_log_detail(&mut self) {
        let len = self.log_view().len();
        let row = self.insights.log_cursor.unwrap_or_else(|| {
            (self.first_visible_log() + self.page_rows())
                .min(len)
                .saturating_sub(1)
        });
        let raw = self.log_view().entry(row).map(|l| l.raw.clone());
        if let Some(raw) = raw {
            self.open_modal(Modal::Detail { raw });
        }
    }

    fn open_modal(&mut self, modal: Modal) {
        self.insights.modal = Some(modal);
        self.insights.modal_scroll = 0;
    }

    fn toggle_insights_pause(&mut self) {
        let now = Instant::now();
        if self.insights.paused {
            self.insights.paused = false;
            self.logs_follow = self.insights.follow_before_pause;
            if self.logs_follow {
                self.logs_scroll = self.log_view().len();
            }
            self.insights.refresh_now(now);
            self.set_toast("resumed");
        } else {
            self.insights.follow_before_pause = self.logs_follow;
            self.logs_scroll = self.first_visible_log();
            self.logs_follow = false;
            self.insights.paused = true;
            self.set_toast("paused — logs keep buffering · Space to resume");
        }
    }

    fn toast_interval(&mut self) {
        let every = self.insights.interval();
        let text = if every.as_millis() < 1000 {
            format!("{}ms", every.as_millis())
        } else {
            format!("{}s", every.as_secs())
        };
        self.set_toast(format!("insights refresh every {text}"));
    }

    fn begin_input(&mut self, kind: InputKind) {
        let text = match kind {
            InputKind::Filter => self.insights.filter_text.clone(),
            InputKind::Search => self.insights.search.clone().unwrap_or_default(),
        };
        self.insights.input = Some(Input { kind, text });
    }

    /// Edit the text being typed; the filter and search update as you type.
    fn edit_input(&mut self, edit: impl FnOnce(&mut String)) {
        let Some(input) = self.insights.input.as_mut() else {
            return;
        };
        edit(&mut input.text);
        let (kind, text) = (input.kind, input.text.clone());
        match kind {
            InputKind::Filter => self.set_log_regex(&text),
            InputKind::Search => self.insights.search = (!text.is_empty()).then_some(text),
        }
    }

    /// Esc while typing clears that filter or search.
    fn cancel_input(&mut self) {
        match self.insights.input.take().map(|i| i.kind) {
            Some(InputKind::Filter) => self.set_log_regex(""),
            Some(InputKind::Search) => self.insights.search = None,
            None => {}
        }
    }

    /// Apply the `/` regex. An incomplete pattern keeps the last valid one.
    fn set_log_regex(&mut self, text: &str) {
        if text.is_empty() {
            self.insights.filter.regex = None;
            self.insights.filter_text.clear();
        } else if let Ok(re) = Regex::new(text) {
            self.insights.filter.regex = Some(re);
            self.insights.filter_text = text.to_string();
        } else {
            return;
        }
        self.refilter_logs();
    }

    fn refilter_logs(&mut self) {
        self.insights.refilter(&self.logs);
        self.insights.log_cursor = None;
        self.visual_select = None;
        if self.logs_follow {
            self.logs_scroll = self.log_view().len();
        } else {
            self.logs_scroll = self
                .logs_scroll
                .min(self.log_view().len().saturating_sub(1));
        }
    }

    fn apply_picker(&mut self) {
        match self.insights.modal.take() {
            Some(Modal::SeverityFilter(p)) => {
                self.insights.filter.hidden = apply_severity(&p);
                let shown = self.insights.filter.hidden.iter().filter(|h| !**h).count();
                self.refilter_logs();
                self.set_toast(format!("{shown}/6 levels shown"));
            }
            Some(Modal::Columns(p)) => self.insights.columns = apply_columns(&p),
            other => self.insights.modal = other,
        }
    }

    fn copy_detail(&mut self, full: bool) {
        let Some(Modal::Detail { raw }) = &self.insights.modal else {
            return;
        };
        let d = detail(raw);
        let (text, what) = if full {
            (clipboard_text(&d), "full log entry")
        } else {
            (d.message.clone(), "message")
        };
        match self.clipboard.set(&text) {
            Ok(_) => self.set_toast(format!("copied {what}")),
            Err(e) => self.set_toast(format!("copy failed: {e}")),
        }
    }
}
