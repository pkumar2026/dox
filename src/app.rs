use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyEvent, KeyEventKind,
    MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use futures_util::StreamExt;
use ratatui::backend::Backend;
use ratatui::layout::Rect;
use ratatui::widgets::TableState;
use ratatui::Terminal;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{error, info, warn};

use crate::clipboard::Clipboard;
use crate::config::Config;
use crate::docker::containers::{self, ContainerRow};
use crate::docker::images::{self, ImageRow};
use crate::docker::networks::{self, NetworkRow};
use crate::docker::volumes::{self, VolumeRow};
use crate::docker::DockerClient;
use crate::events::{self, Action, Mode};
use crate::ui::logs::{LogBuffer, Selection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Containers,
    Images,
    Volumes,
    Networks,
}

impl Panel {
    pub fn index(self) -> usize {
        match self {
            Self::Containers => 0,
            Self::Images => 1,
            Self::Volumes => 2,
            Self::Networks => 3,
        }
    }
    pub fn from_index(i: usize) -> Option<Self> {
        match i {
            0 => Some(Self::Containers),
            1 => Some(Self::Images),
            2 => Some(Self::Volumes),
            3 => Some(Self::Networks),
            _ => None,
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::Containers => Self::Images,
            Self::Images => Self::Volumes,
            Self::Volumes => Self::Networks,
            Self::Networks => Self::Containers,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            Self::Containers => Self::Networks,
            Self::Images => Self::Containers,
            Self::Volumes => Self::Images,
            Self::Networks => Self::Volumes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusArea {
    List,
    Detail,
}

#[derive(Debug, Clone)]
pub struct PendingConfirm {
    pub prompt: String,
    pub action: Box<ConfirmAction>,
}

#[derive(Debug, Clone)]
pub enum ConfirmAction {
    DeleteContainer {
        id: String,
        name: String,
        force: bool,
    },
    StopContainer {
        id: String,
        name: String,
    },
    StartContainer {
        id: String,
        name: String,
    },
    RestartContainer {
        id: String,
        name: String,
    },
    DeleteImage {
        id: String,
        label: String,
        force: bool,
    },
    DeleteVolume {
        name: String,
        force: bool,
    },
    DeleteNetwork {
        id: String,
        name: String,
    },
    PruneImages,
    PruneVolumes,
    PruneNetworks,
}

/// Messages produced asynchronously and folded into App state on each tick.
#[derive(Debug)]
pub enum AppMsg {
    LogChunk(String),
    LogEnded(Option<String>),
    LogStarted {
        container_id: String,
        container_name: String,
    },
    ActionDone {
        description: String,
        result: Result<String, String>,
    },
    /// Result of a background list refresh. `None` for a list means it was not
    /// fetched this round (see `spawn_refresh`), not that it came back empty.
    Lists(Box<ListSnapshot>),
    Refresh,
}

/// One round of list data, fetched off the event loop.
#[derive(Debug, Default)]
pub struct ListSnapshot {
    pub containers: Option<Vec<ContainerRow>>,
    pub images: Option<Vec<ImageRow>>,
    pub volumes: Option<Vec<VolumeRow>>,
    pub networks: Option<Vec<NetworkRow>>,
}

/// Lifecycle of the current log stream — used to render a meaningful empty-state
/// hint instead of an indefinite "(waiting…)".
#[derive(Debug, Clone)]
pub enum LogStreamState {
    Idle,
    Starting { container_name: String },
    Streaming { container_name: String, chunks: u64 },
    Errored { container_name: String, reason: String },
    Ended { container_name: String },
}

pub struct App {
    pub client: DockerClient,
    pub cfg: Config,
    pub panel: Panel,
    pub focus: FocusArea,
    pub mode: Mode,

    pub containers: Vec<ContainerRow>,
    pub images: Vec<ImageRow>,
    pub volumes: Vec<VolumeRow>,
    pub networks: Vec<NetworkRow>,

    pub containers_state: TableState,
    pub images_state: TableState,
    pub volumes_state: TableState,
    pub networks_state: TableState,

    pub logs: LogBuffer,
    pub logs_follow: bool,
    pub logs_scroll: usize,
    /// Height of the log viewport last time it was drawn — used to seed
    /// scroll position when transitioning from follow to manual scroll.
    pub last_log_height: usize,
    /// Interior rect of the log viewport as last drawn. Used to hit-test mouse
    /// events against the log pane instead of re-deriving the layout.
    pub last_log_inner: Rect,
    pub log_task: Option<JoinHandle<()>>,
    pub log_container_id: Option<String>,
    /// Name of the container whose logs are currently streaming. Tracks the
    /// *active* stream (set on an explicit open), not the highlighted row, so
    /// the log pane title stays correct while you browse the container list.
    pub log_container_name: Option<String>,
    pub log_stream: LogStreamState,
    pub visual_select: Option<Selection>,

    pub filter: String,
    pub pending: Option<PendingConfirm>,
    pub toast: Option<(String, Instant)>,
    pub should_quit: bool,

    pub clipboard: Box<dyn Clipboard>,
    pub msg_tx: mpsc::UnboundedSender<AppMsg>,

    /// Live mouse-capture state. Mirrors `cfg.mouse` initially; flipped by `m`.
    pub mouse_on: bool,

    /// Percentage of body height occupied by the resource panels area (top).
    /// Logs get `100 - top_split_percent`. Clamped to [10, 70] by `clamp_split`.
    pub top_split_percent: u16,

    /// Body area last drawn — used by mouse drag-to-resize on the splitter.
    pub last_body_y: u16,
    pub last_body_height: u16,
    /// True while the user is dragging the splitter row to resize.
    pub resizing_split: bool,

    /// Geometry recorded by the last draw, so `hit_test` reasons about the
    /// rects that were actually painted rather than recomputing the layout.
    /// Row of the header tab bar.
    pub last_header_y: u16,
    /// Per-tab `[start, end)` column range within the header tab bar.
    pub last_tab_spans: [(u16, u16); 4],
    /// Interior rect of each resource panel (row 0 of each is the table header).
    pub last_panel_inner: [Rect; 4],

    /// Filtered row indices per panel, in `Panel::index()` order. Rebuilt only
    /// by `recompute_visible`; see `visible_containers` for why.
    visible: [Vec<usize>; 4],
    /// Set while a background list refresh is in flight, so ticks don't pile up
    /// overlapping requests against a slow daemon. Holds the start time rather
    /// than a bool so a task that dies without replying cannot wedge refreshes
    /// off for the rest of the session.
    refresh_started: Option<Instant>,
}

impl App {
    pub fn new(
        client: DockerClient,
        cfg: Config,
        clipboard: Box<dyn Clipboard>,
        msg_tx: mpsc::UnboundedSender<AppMsg>,
    ) -> Self {
        let logs = LogBuffer::new(cfg.log_buffer_lines);
        let mut containers_state = TableState::default();
        containers_state.select(Some(0));
        let mut images_state = TableState::default();
        images_state.select(Some(0));
        let mut volumes_state = TableState::default();
        volumes_state.select(Some(0));
        let mut networks_state = TableState::default();
        networks_state.select(Some(0));

        let mouse_on = cfg.mouse;
        Self {
            client,
            cfg,
            panel: Panel::Containers,
            focus: FocusArea::List,
            mode: Mode::Normal,
            containers: vec![],
            images: vec![],
            volumes: vec![],
            networks: vec![],
            containers_state,
            images_state,
            volumes_state,
            networks_state,
            logs,
            logs_follow: true,
            logs_scroll: 0,
            last_log_height: 20,
            last_log_inner: Rect::default(),
            log_task: None,
            log_container_id: None,
            log_container_name: None,
            log_stream: LogStreamState::Idle,
            visual_select: None,
            filter: String::new(),
            pending: None,
            toast: None,
            should_quit: false,
            clipboard,
            msg_tx,
            mouse_on,
            top_split_percent: 20,
            last_body_y: 0,
            last_body_height: 0,
            resizing_split: false,
            last_header_y: 0,
            last_tab_spans: [(0, 0); 4],
            last_panel_inner: [Rect::default(); 4],
            visible: Default::default(),
            refresh_started: None,
        }
    }

    pub fn adjust_split(&mut self, delta: i16) {
        let cur = self.top_split_percent as i16 + delta;
        self.top_split_percent = clamp_split(cur);
        self.set_toast(format!("panels: {}% / logs: {}%", self.top_split_percent, 100 - self.top_split_percent));
    }

    pub fn reset_split(&mut self) {
        self.top_split_percent = 20;
        self.set_toast("panel split reset to 20%");
    }

    pub fn mode_is_confirm(&self) -> bool {
        matches!(self.mode, Mode::Confirm)
    }
    pub fn mode_is_filtering(&self) -> bool {
        matches!(self.mode, Mode::Filtering)
    }
    pub fn mode_is_help(&self) -> bool {
        matches!(self.mode, Mode::Help)
    }

    pub fn confirm_prompt(&self) -> Option<&str> {
        self.pending.as_ref().map(|p| p.prompt.as_str())
    }

    pub fn client_socket_short(&self) -> String {
        let s = &self.client.info.socket;
        if let Some(idx) = s.rfind('/') {
            format!("●{}", &s[idx..])
        } else {
            s.clone()
        }
    }

    pub fn client_version_short(&self) -> String {
        format!("v{}", self.client.info.server_version)
    }

    pub fn toast_text(&self) -> Option<&str> {
        self.toast
            .as_ref()
            .filter(|(_, t)| t.elapsed() < Duration::from_millis(3500))
            .map(|(s, _)| s.as_str())
    }

    pub fn set_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub fn current_panel_count(&self) -> usize {
        self.visible_len(self.panel)
    }

    pub fn filter_query(&self) -> &str {
        &self.filter
    }

    /// Indices into `self.containers` that pass the current filter. Cached by
    /// `recompute_visible` so filtering runs once per data/filter change rather
    /// than on every call — the draw path alone used to call this ~8x per frame.
    pub fn visible_containers(&self) -> &[usize] {
        &self.visible[0]
    }
    pub fn visible_images(&self) -> &[usize] {
        &self.visible[1]
    }
    pub fn visible_volumes(&self) -> &[usize] {
        &self.visible[2]
    }
    pub fn visible_networks(&self) -> &[usize] {
        &self.visible[3]
    }

    pub fn visible_len(&self, panel: Panel) -> usize {
        self.visible[panel.index()].len()
    }

    /// Rebuild the filtered index lists. Call after replacing any list or
    /// changing the filter — nothing else invalidates them.
    pub fn recompute_visible(&mut self) {
        let q = self.filter.to_lowercase();
        self.visible[0] = matching_indices(&self.containers, &q, |c| {
            [c.name.as_str(), c.image.as_str()]
        });
        self.visible[1] = matching_indices(&self.images, &q, |i| [i.repo_tag.as_str(), ""]);
        self.visible[2] = matching_indices(&self.volumes, &q, |v| [v.name.as_str(), ""]);
        self.visible[3] = matching_indices(&self.networks, &q, |n| [n.name.as_str(), ""]);
    }

    pub fn selected_container(&self) -> Option<ContainerRow> {
        let idx = view_to_row(&self.visible[0], self.containers_state.selected())?;
        self.containers.get(idx).cloned()
    }
    pub fn selected_container_id(&self) -> Option<String> {
        let idx = view_to_row(&self.visible[0], self.containers_state.selected())?;
        self.containers.get(idx).map(|c| c.id.clone())
    }
    /// Name of the container whose logs are currently on screen — the active
    /// stream, which is independent of the highlighted row.
    pub fn active_log_label(&self) -> String {
        self.log_container_name
            .clone()
            .unwrap_or_else(|| "(none)".into())
    }
    pub fn selected_image(&self) -> Option<ImageRow> {
        let idx = view_to_row(&self.visible[1], self.images_state.selected())?;
        self.images.get(idx).cloned()
    }
    pub fn selected_volume(&self) -> Option<VolumeRow> {
        let idx = view_to_row(&self.visible[2], self.volumes_state.selected())?;
        self.volumes.get(idx).cloned()
    }
    pub fn selected_network(&self) -> Option<NetworkRow> {
        let idx = view_to_row(&self.visible[3], self.networks_state.selected())?;
        self.networks.get(idx).cloned()
    }

    fn current_state_mut(&mut self) -> &mut TableState {
        match self.panel {
            Panel::Containers => &mut self.containers_state,
            Panel::Images => &mut self.images_state,
            Panel::Volumes => &mut self.volumes_state,
            Panel::Networks => &mut self.networks_state,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.current_panel_count();
        if len == 0 {
            return;
        }
        let state = self.current_state_mut();
        let cur = state.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, len as isize - 1) as usize;
        state.select(Some(next));
    }

    fn select_panel(&mut self, panel: Panel) {
        self.panel = panel;
        self.focus = FocusArea::List;
        self.mode = Mode::Normal;
        self.visual_select = None;
        if matches!(self.panel, Panel::Containers) && self.log_container_id.is_none() {
            self.open_selected_logs();
        }
    }

    fn select_panel_index(&mut self, i: usize) {
        if let Some(panel) = Panel::from_index(i) {
            self.select_panel(panel);
        }
    }

    /// Esc in normal mode. Unwinds one layer of state per press, innermost
    /// first, so Esc always means "back" the way it does in lazygit — instead
    /// of only ever leaving the log pane.
    fn go_back(&mut self) {
        if self.visual_select.is_some() {
            self.visual_select = None;
            self.mode = Mode::Normal;
        } else if matches!(self.focus, FocusArea::Detail) {
            self.focus = FocusArea::List;
        } else if !self.filter.is_empty() {
            self.filter.clear();
            self.refilter();
            self.set_toast("filter cleared");
        }
    }

    fn jump(&mut self, top: bool) {
        let len = self.current_panel_count();
        if len == 0 {
            return;
        }
        self.current_state_mut()
            .select(Some(if top { 0 } else { len - 1 }));
    }

    pub fn ingest_msg(&mut self, msg: AppMsg) {
        match msg {
            AppMsg::LogChunk(chunk) => {
                self.logs.extend_chunk(&chunk);
                if self.logs_follow {
                    self.logs_scroll = self.logs.len();
                }
                if let LogStreamState::Starting { container_name }
                | LogStreamState::Streaming { container_name, .. } =
                    self.log_stream.clone()
                {
                    let chunks = match &self.log_stream {
                        LogStreamState::Streaming { chunks, .. } => *chunks + 1,
                        _ => 1,
                    };
                    self.log_stream = LogStreamState::Streaming {
                        container_name,
                        chunks,
                    };
                }
            }
            AppMsg::LogStarted {
                container_id,
                container_name,
            } => {
                info!(container_id = %container_id, name = %container_name, "log stream task started");
                self.log_stream = LogStreamState::Starting { container_name };
            }
            AppMsg::LogEnded(error) => {
                let container_name = match &self.log_stream {
                    LogStreamState::Starting { container_name }
                    | LogStreamState::Streaming { container_name, .. }
                    | LogStreamState::Errored { container_name, .. }
                    | LogStreamState::Ended { container_name } => container_name.clone(),
                    LogStreamState::Idle => "(unknown)".to_string(),
                };
                match error {
                    Some(reason) => {
                        warn!(container = %container_name, reason = %reason, "log stream errored");
                        self.log_stream = LogStreamState::Errored {
                            container_name,
                            reason,
                        };
                    }
                    None => {
                        info!(container = %container_name, "log stream ended cleanly");
                        self.log_stream = LogStreamState::Ended { container_name };
                    }
                }
            }
            AppMsg::ActionDone {
                description,
                result,
            } => match result {
                Ok(out) => {
                    info!(action = %description, "{}", out);
                    self.set_toast(format!("{} ✓ {}", description, out));
                }
                Err(e) => {
                    error!(action = %description, "failed: {}", e);
                    self.set_toast(format!("{} failed: {}", description, e));
                }
            },
            AppMsg::Lists(snap) => {
                self.refresh_started = None;
                if snap.containers.is_none()
                    && snap.images.is_none()
                    && snap.volumes.is_none()
                    && snap.networks.is_none()
                {
                    self.set_toast("docker refresh failed — showing last known state");
                }
                self.apply_lists(*snap);
            }
            AppMsg::Refresh => self.spawn_refresh(true),
        }
    }

    /// Fetch lists on a background task and deliver them as `AppMsg::Lists`.
    /// Never awaited by the event loop — a refresh against a slow daemon takes
    /// ~100ms here, and awaiting it inline stalls every keystroke for that long.
    ///
    /// `full` also refreshes images/volumes/networks; those change only on user
    /// action (which already posts `AppMsg::Refresh`), so the periodic tick
    /// asks for containers alone most of the time.
    pub fn spawn_refresh(&mut self, full: bool) {
        // Re-arm if a previous refresh never replied (task panicked or was
        // dropped); otherwise a single lost reply would freeze the lists.
        if self
            .refresh_started
            .is_some_and(|t| t.elapsed() < REFRESH_STALE_AFTER)
        {
            return;
        }
        self.refresh_started = Some(Instant::now());
        let client = self.client.clone();
        let tx = self.msg_tx.clone();
        tokio::spawn(async move {
            let mut snap = ListSnapshot::default();
            if full {
                let (c, i, v, n) = tokio::join!(
                    containers::list(&client),
                    images::list(&client),
                    volumes::list(&client),
                    networks::list(&client),
                );
                snap.containers = c.ok();
                snap.images = i.ok();
                snap.volumes = v.ok();
                snap.networks = n.ok();
            } else {
                snap.containers = containers::list(&client).await.ok();
            }
            let _ = tx.send(AppMsg::Lists(Box::new(snap)));
        });
    }

    /// Blocking refresh used once at startup so the first frame has data.
    pub async fn refresh_all(&mut self) -> Result<()> {
        let (c, i, v, n) = tokio::try_join!(
            containers::list(&self.client),
            images::list(&self.client),
            volumes::list(&self.client),
            networks::list(&self.client),
        )?;
        self.apply_lists(ListSnapshot {
            containers: Some(c),
            images: Some(i),
            volumes: Some(v),
            networks: Some(n),
        });
        Ok(())
    }

    fn apply_lists(&mut self, snap: ListSnapshot) {
        if let Some(c) = snap.containers {
            self.containers = c;
        }
        if let Some(i) = snap.images {
            self.images = i;
        }
        if let Some(v) = snap.volumes {
            self.volumes = v;
        }
        if let Some(n) = snap.networks {
            self.networks = n;
        }
        self.recompute_visible();
        self.clamp_all_selections();
        // Auto-open logs for the highlighted container only when nothing is
        // streaming yet (first launch). After that the stream is decoupled from
        // selection — it changes only on an explicit open (Enter / l).
        if matches!(self.panel, Panel::Containers) && self.log_container_id.is_none() {
            self.open_selected_logs();
        }
    }

    /// The filter changed: rebuild the cached index lists, then re-clamp.
    fn refilter(&mut self) {
        self.recompute_visible();
        self.clamp_all_selections();
    }

    fn clamp_all_selections(&mut self) {
        for panel in [
            Panel::Containers,
            Panel::Images,
            Panel::Volumes,
            Panel::Networks,
        ] {
            let len = self.visible_len(panel);
            let state = match panel {
                Panel::Containers => &mut self.containers_state,
                Panel::Images => &mut self.images_state,
                Panel::Volumes => &mut self.volumes_state,
                Panel::Networks => &mut self.networks_state,
            };
            if len == 0 {
                state.select(None);
            } else {
                let cur = state.selected().unwrap_or(0);
                if cur >= len {
                    state.select(Some(len - 1));
                } else if state.selected().is_none() {
                    state.select(Some(0));
                }
            }
        }
    }

    pub fn handle_mouse(&mut self, m: MouseEvent) {
        if !self.mouse_on {
            return;
        }
        match m.kind {
            MouseEventKind::ScrollUp => self.wheel(m.column, m.row, -3),
            MouseEventKind::ScrollDown => self.wheel(m.column, m.row, 3),
            MouseEventKind::Down(MouseButton::Left) => match self.hit_test(m.column, m.row) {
                Hit::Tab(i) | Hit::PanelBody(i) => self.select_panel_index(i),
                Hit::PanelRow { panel, row } => {
                    self.select_panel_index(panel);
                    self.current_state_mut().select(Some(row));
                }
                Hit::Logs => self.mouse_anchor_logs(m.column, m.row),
                Hit::Splitter => {
                    self.resizing_split = true;
                    self.update_split_from_y(m.row);
                }
                Hit::Nothing => {}
            },
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.resizing_split {
                    self.update_split_from_y(m.row);
                } else if self.visual_select.is_some() {
                    self.mouse_extend_logs(m.column, m.row);
                }
            }
            MouseEventKind::Up(MouseButton::Left) if self.resizing_split => {
                self.resizing_split = false;
                self.set_toast(format!(
                    "panels: {}% / logs: {}%",
                    self.top_split_percent,
                    100 - self.top_split_percent
                ));
            }
            _ => {}
        }
    }

    /// The wheel acts on whatever is under the cursor: a resource list if the
    /// pointer is over one, the log pane otherwise.
    fn wheel(&mut self, col: u16, row: u16, delta: isize) {
        match self.hit_test(col, row) {
            Hit::PanelRow { panel, .. } | Hit::PanelBody(panel) | Hit::Tab(panel) => {
                self.select_panel_index(panel);
                self.move_selection(delta);
            }
            _ => self.scroll_logs(delta),
        }
    }

    /// Resolve a screen coordinate against the rects recorded by the last draw.
    /// Interiors are tested before the splitter so the resize hit zone can keep
    /// its ±1 row tolerance without swallowing the first log line or the last
    /// visible table row.
    fn hit_test(&self, col: u16, row: u16) -> Hit {
        if row == self.last_header_y {
            for (i, (x0, x1)) in self.last_tab_spans.iter().enumerate() {
                if col >= *x0 && col < *x1 {
                    return Hit::Tab(i);
                }
            }
            return Hit::Nothing;
        }

        for i in 0..4 {
            let inner = self.last_panel_inner[i];
            if !contains(inner, col, row) {
                continue;
            }
            // Row 0 of a panel interior is the table's own header row.
            if row == inner.y {
                return Hit::PanelBody(i);
            }
            let offset = self.panel_state(i).offset();
            let idx = offset + (row - inner.y - 1) as usize;
            return if idx < self.panel_count(i) {
                Hit::PanelRow { panel: i, row: idx }
            } else {
                Hit::PanelBody(i)
            };
        }

        if contains(self.last_log_inner, col, row) {
            return Hit::Logs;
        }
        if self.is_on_splitter(row) {
            return Hit::Splitter;
        }
        Hit::Nothing
    }

    fn panel_state(&self, i: usize) -> &TableState {
        match Panel::from_index(i).unwrap_or(Panel::Containers) {
            Panel::Containers => &self.containers_state,
            Panel::Images => &self.images_state,
            Panel::Volumes => &self.volumes_state,
            Panel::Networks => &self.networks_state,
        }
    }

    fn panel_count(&self, i: usize) -> usize {
        self.visible_len(Panel::from_index(i).unwrap_or(Panel::Containers))
    }

    /// Returns true if `row` is on (or within ±1 of) the splitter line — the
    /// top border of the logs block.
    fn is_on_splitter(&self, row: u16) -> bool {
        if self.last_body_height == 0 {
            return false;
        }
        let split_row =
            self.last_body_y + (self.last_body_height * self.top_split_percent / 100);
        row.abs_diff(split_row) <= 1
    }

    fn update_split_from_y(&mut self, row: u16) {
        if self.last_body_height == 0 {
            return;
        }
        let offset = row.saturating_sub(self.last_body_y) as i16;
        let pct = (offset * 100) / self.last_body_height as i16;
        self.top_split_percent = clamp_split(pct);
    }

    /// Map a screen row inside the log pane to a log buffer index.
    fn log_line_at(&self, col: u16, row: u16) -> Option<usize> {
        if !contains(self.last_log_inner, col, row) {
            return None;
        }
        let total = self.logs.len();
        if total == 0 {
            return None;
        }
        let offset_within_pane = (row - self.last_log_inner.y) as usize;
        // Mirror logs::visible_range to find the first-rendered line index.
        let height = self.last_log_height.max(1);
        let first = if self.logs_follow {
            total.saturating_sub(height)
        } else {
            self.logs_scroll.min(total.saturating_sub(1))
        };
        let idx = first + offset_within_pane;
        if idx < total { Some(idx) } else { None }
    }

    fn mouse_anchor_logs(&mut self, col: u16, row: u16) {
        let Some(idx) = self.log_line_at(col, row) else {
            return;
        };
        self.focus = FocusArea::Detail;
        self.logs_follow = false;
        self.visual_select = Some(Selection {
            anchor: idx,
            cursor: idx,
        });
        self.mode = Mode::Visual;
        self.set_toast("drag to extend · y to copy · Esc to cancel");
    }

    fn mouse_extend_logs(&mut self, col: u16, row: u16) {
        let Some(idx) = self.log_line_at(col, row) else {
            return;
        };
        if let Some(sel) = self.visual_select.as_mut() {
            sel.cursor = idx;
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        let Some(action) = events::map(key, self.mode) else {
            return;
        };
        self.apply_action(action);
    }

    fn apply_action(&mut self, action: Action) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::NextPanel => self.select_panel(self.panel.next()),
            Action::PrevPanel => self.select_panel(self.panel.prev()),
            Action::SelectPanel(i) => self.select_panel_index(i),
            Action::Back => self.go_back(),
            Action::SelectNext => {
                if matches!(self.focus, FocusArea::Detail) {
                    self.scroll_logs(1);
                    if let Some(sel) = self.visual_select.as_mut() {
                        sel.cursor = sel
                            .cursor
                            .saturating_add(1)
                            .min(self.logs.len().saturating_sub(1));
                    }
                } else {
                    // Browsing the list no longer switches the log stream — the
                    // current stream keeps flowing. Press Enter / l to switch.
                    self.move_selection(1);
                }
            }
            Action::SelectPrev => {
                if matches!(self.focus, FocusArea::Detail) {
                    self.scroll_logs(-1);
                    if let Some(sel) = self.visual_select.as_mut() {
                        sel.cursor = sel.cursor.saturating_sub(1);
                    }
                } else {
                    self.move_selection(-1);
                }
            }
            Action::SelectTop => {
                if matches!(self.focus, FocusArea::Detail) {
                    self.logs_scroll = 0;
                    self.logs_follow = false;
                } else {
                    self.jump(true);
                }
            }
            Action::SelectBottom => {
                if matches!(self.focus, FocusArea::Detail) {
                    self.logs_scroll = self.logs.len();
                    self.logs_follow = true;
                } else {
                    self.jump(false);
                }
            }
            Action::PageDown => {
                self.scroll_logs(20);
                self.extend_visual(20);
            }
            Action::PageUp => {
                self.scroll_logs(-20);
                self.extend_visual(-20);
            }
            Action::LogScrollDown => {
                self.scroll_logs(1);
                self.extend_visual(1);
            }
            Action::LogScrollUp => {
                self.scroll_logs(-1);
                self.extend_visual(-1);
            }
            Action::LogJumpTop => {
                self.logs_follow = false;
                self.logs_scroll = 0;
            }
            Action::LogJumpBottom => {
                self.logs_follow = true;
                self.logs_scroll = self.logs.len();
            }
            Action::FocusDetail => {
                if matches!(self.panel, Panel::Containers) {
                    self.focus = FocusArea::Detail;
                    self.open_selected_logs();
                }
            }
            Action::UnfocusDetail => {
                self.focus = FocusArea::List;
                self.visual_select = None;
                self.mode = Mode::Normal;
            }
            Action::ToggleLogs => {
                if matches!(self.panel, Panel::Containers) {
                    self.focus = FocusArea::Detail;
                    self.open_selected_logs();
                }
            }
            Action::ToggleFollow => {
                // `f` always means "go live": jump to bottom + enable follow.
                // Pressing it while already following is a harmless no-op.
                // To pause follow, scroll up (any of K, PageUp, wheel, k).
                self.logs_follow = true;
                self.logs_scroll = self.logs.len();
            }
            Action::EnterVisualMode => {
                if self.logs.is_empty() {
                    self.set_toast("no logs to select");
                } else {
                    // Auto-focus the log pane so arrows extend selection here.
                    self.focus = FocusArea::Detail;
                    self.logs_follow = false;
                    // Anchor at the bottom of what's currently on screen.
                    let total = self.logs.len();
                    let pos = if self.logs_scroll == 0 {
                        total.saturating_sub(1)
                    } else {
                        (self.logs_scroll + self.last_log_height.saturating_sub(1))
                            .min(total.saturating_sub(1))
                    };
                    self.visual_select = Some(Selection {
                        anchor: pos,
                        cursor: pos,
                    });
                    self.mode = Mode::Visual;
                    self.set_toast("VISUAL: ↑↓ extend, y to yank, Esc to cancel");
                }
            }
            Action::YankSelection => {
                let text = if let Some(sel) = self.visual_select {
                    let (lo, hi) = sel.range();
                    self.logs.collect_range(lo, hi)
                } else {
                    self.logs.collect_all()
                };
                tracing::info!(
                    bytes = text.len(),
                    lines = text.lines().count(),
                    "yank attempt"
                );
                if text.is_empty() {
                    self.set_toast("nothing to copy");
                } else {
                    let lines = text.lines().count();
                    match self.clipboard.set(&text) {
                        Ok(_) => {
                            tracing::info!(lines, "yank ok");
                            self.set_toast(format!("yanked {} line(s) → clipboard", lines));
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "yank failed");
                            self.set_toast(format!("yank failed: {}", e));
                        }
                    }
                }
                self.visual_select = None;
                self.mode = Mode::Normal;
                self.focus = FocusArea::List;
            }
            Action::StopContainer => {
                if let Some(c) = self.selected_container() {
                    self.enter_confirm(
                        format!("Stop container {}?", c.name),
                        ConfirmAction::StopContainer {
                            id: c.id,
                            name: c.name,
                        },
                    );
                }
            }
            Action::StartContainer => {
                if let Some(c) = self.selected_container() {
                    self.enter_confirm(
                        format!("Start container {}?", c.name),
                        ConfirmAction::StartContainer {
                            id: c.id,
                            name: c.name,
                        },
                    );
                }
            }
            Action::RestartContainer => {
                if let Some(c) = self.selected_container() {
                    self.enter_confirm(
                        format!("Restart container {}?", c.name),
                        ConfirmAction::RestartContainer {
                            id: c.id,
                            name: c.name,
                        },
                    );
                }
            }
            Action::DeleteSelected => self.enqueue_delete(),
            Action::PruneCurrentPanel => self.enqueue_prune(),
            Action::BeginFilter => {
                self.mode = Mode::Filtering;
                self.filter.clear();
                self.refilter();
            }
            Action::AppendFilter(c) => {
                self.filter.push(c);
                self.refilter();
            }
            Action::BackspaceFilter => {
                self.filter.pop();
                self.refilter();
            }
            Action::CommitFilter | Action::CancelFilter => {
                if matches!(action, Action::CancelFilter) {
                    self.filter.clear();
                }
                self.mode = Mode::Normal;
                self.refilter();
            }
            Action::ShowHelp => self.mode = Mode::Help,
            Action::ToggleMouseCapture => self.toggle_mouse_capture(),
            Action::GrowPanels => self.adjust_split(5),
            Action::ShrinkPanels => self.adjust_split(-5),
            Action::ResetPanels => self.reset_split(),
            Action::RestartLogStream => {
                // Restart the active stream (spawn_log_stream force-reopens even
                // for the same container). If nothing is active yet, fall back to
                // opening the highlighted row.
                match (self.log_container_id.clone(), self.log_container_name.clone()) {
                    (Some(id), Some(name)) => self.spawn_log_stream(id, name),
                    _ => self.open_selected_logs(),
                }
                self.set_toast("log stream restarted");
            }
            Action::Confirm => self.run_confirmed(),
            Action::Cancel => {
                self.pending = None;
                self.mode = Mode::Normal;
            }
        }

        // recompute selection clamps for log buffer
        if let Some(sel) = self.visual_select.as_mut() {
            let max = self.logs.len().saturating_sub(1);
            sel.cursor = sel.cursor.min(max);
            sel.anchor = sel.anchor.min(max);
        }
    }

    fn toggle_mouse_capture(&mut self) {
        let result = if self.mouse_on {
            execute!(std::io::stdout(), DisableMouseCapture)
        } else {
            execute!(std::io::stdout(), EnableMouseCapture)
        };
        match result {
            Ok(()) => {
                self.mouse_on = !self.mouse_on;
                if self.mouse_on {
                    self.set_toast("mouse ON · wheel scrolls · drag-select disabled");
                } else {
                    self.set_toast("mouse OFF · native drag-select + copy works");
                }
                tracing::info!(mouse_on = self.mouse_on, "mouse capture toggled");
            }
            Err(e) => {
                self.set_toast(format!("mouse toggle failed: {}", e));
                tracing::error!(error = %e, "mouse toggle failed");
            }
        }
    }

    fn extend_visual(&mut self, delta: isize) {
        let Some(sel) = self.visual_select.as_mut() else {
            return;
        };
        let max = self.logs.len().saturating_sub(1);
        if delta >= 0 {
            sel.cursor = (sel.cursor + delta as usize).min(max);
        } else {
            sel.cursor = sel.cursor.saturating_sub((-delta) as usize);
        }
    }

    fn scroll_logs(&mut self, delta: isize) {
        let before = self.logs_scroll;
        let before_follow = self.logs_follow;
        let max = self.logs.len();
        let h = self.last_log_height.max(1);

        // Seed scroll to current bottom when transitioning out of follow on scroll-up.
        if self.logs_follow && delta < 0 {
            self.logs_scroll = max.saturating_sub(h);
            self.logs_follow = false;
        }
        if delta > 0 {
            self.logs_scroll = (self.logs_scroll + delta as usize).min(max);
        } else if delta < 0 {
            let abs = (-delta) as usize;
            self.logs_scroll = self.logs_scroll.saturating_sub(abs);
            self.logs_follow = false;
        }

        // Auto-resume follow when scroll reaches (or passes) the bottom — matches
        // less / lazygit / k9s. Use a small 2-line tolerance to avoid bouncing.
        if !self.logs_follow && self.logs_scroll + h + 2 >= max && max > 0 {
            self.logs_follow = true;
            self.logs_scroll = max;
        }

        tracing::debug!(
            delta,
            before,
            after = self.logs_scroll,
            total = max,
            height = h,
            was_follow = before_follow,
            follow = self.logs_follow,
            "scroll_logs"
        );
    }

    fn enter_confirm(&mut self, prompt: String, action: ConfirmAction) {
        self.pending = Some(PendingConfirm {
            prompt,
            action: Box::new(action),
        });
        self.mode = Mode::Confirm;
    }

    fn enqueue_delete(&mut self) {
        match self.panel {
            Panel::Containers => {
                if let Some(c) = self.selected_container() {
                    let force = c.state.eq_ignore_ascii_case("running");
                    let prompt = if force {
                        format!("Force-delete RUNNING container {}?", c.name)
                    } else {
                        format!("Delete container {}?", c.name)
                    };
                    self.enter_confirm(
                        prompt,
                        ConfirmAction::DeleteContainer {
                            id: c.id,
                            name: c.name,
                            force,
                        },
                    );
                }
            }
            Panel::Images => {
                if let Some(i) = self.selected_image() {
                    let label = i.repo_tag.clone();
                    let id = i.id.clone();
                    self.enter_confirm(
                        format!("Delete image {}?", label),
                        ConfirmAction::DeleteImage {
                            id,
                            label,
                            force: false,
                        },
                    );
                }
            }
            Panel::Volumes => {
                if let Some(v) = self.selected_volume() {
                    let name = v.name.clone();
                    self.enter_confirm(
                        format!("Delete volume {}?", name),
                        ConfirmAction::DeleteVolume { name, force: false },
                    );
                }
            }
            Panel::Networks => {
                if let Some(n) = self.selected_network() {
                    if networks::is_protected(&n.name) {
                        self.set_toast(format!("{} is a built-in network", n.name));
                        return;
                    }
                    self.enter_confirm(
                        format!("Delete network {}?", n.name),
                        ConfirmAction::DeleteNetwork {
                            id: n.id,
                            name: n.name,
                        },
                    );
                }
            }
        }
    }

    fn enqueue_prune(&mut self) {
        let (prompt, action) = match self.panel {
            Panel::Images => ("Prune dangling images?".into(), ConfirmAction::PruneImages),
            Panel::Volumes => ("Prune unused volumes?".into(), ConfirmAction::PruneVolumes),
            Panel::Networks => (
                "Prune unused networks?".into(),
                ConfirmAction::PruneNetworks,
            ),
            Panel::Containers => {
                self.set_toast("prune not supported on Containers panel");
                return;
            }
        };
        self.enter_confirm(prompt, action);
    }

    fn run_confirmed(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        self.mode = Mode::Normal;
        let client = self.client.clone();
        let tx = self.msg_tx.clone();
        let action = *pending.action;
        let desc = describe(&action);
        info!(action = %desc, "dispatching");
        tokio::spawn(async move {
            let outcome = run_action(&client, action)
                .await
                .map_err(|e| format!("{:#}", e));
            let _ = tx.send(AppMsg::ActionDone {
                description: desc,
                result: outcome,
            });
            let _ = tx.send(AppMsg::Refresh);
        });
    }

    /// Switch the log stream to the currently-highlighted container. No-op if
    /// that container is already the active stream, so re-opening it doesn't
    /// wipe the buffer. This is the only selection-driven entry point — it runs
    /// on an explicit open (Enter / l / first launch), never on cursor movement.
    fn open_selected_logs(&mut self) {
        let Some(container) = self.selected_container() else {
            return;
        };
        if self.log_container_id.as_deref() == Some(&container.id) {
            return;
        }
        self.spawn_log_stream(container.id, container.name);
    }

    /// (Re)start the log stream for a specific container: abort any running
    /// task, reset the buffer to follow-at-bottom, and spawn a fresh reader.
    fn spawn_log_stream(&mut self, id: String, name: String) {
        if let Some(handle) = self.log_task.take() {
            handle.abort();
        }
        self.logs.clear();
        self.logs_scroll = 0;
        self.logs_follow = true;
        self.log_container_id = Some(id.clone());
        self.log_container_name = Some(name.clone());
        self.log_stream = LogStreamState::Starting {
            container_name: name.clone(),
        };

        let docker = self.client.raw().clone();
        let tx = self.msg_tx.clone();
        let tail = self.cfg.log_tail_initial;
        let container_name = name;
        // Tell the UI a new stream is starting so it can show a meaningful hint.
        let _ = tx.send(AppMsg::LogStarted {
            container_id: id.clone(),
            container_name: container_name.clone(),
        });
        let handle = tokio::spawn(async move {
            tracing::info!(container = %container_name, id = %id, tail, "spawning log stream task");
            let mut stream = Box::pin(containers::logs(&docker, &id, tail));
            let mut last_err: Option<String> = None;
            while let Some(item) = stream.next().await {
                match item {
                    Ok(chunk) => {
                        if tx.send(AppMsg::LogChunk(chunk)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let msg = format!("{}", e);
                        tracing::warn!(container = %container_name, error = %msg, "log stream error");
                        last_err = Some(msg);
                        break;
                    }
                }
            }
            tracing::info!(container = %container_name, "log stream loop exited");
            let _ = tx.send(AppMsg::LogEnded(last_err));
        });
        self.log_task = Some(handle);
    }
}

/// How long to wait before assuming an in-flight refresh will never reply.
const REFRESH_STALE_AFTER: Duration = Duration::from_secs(10);

pub fn clamp_split(p: i16) -> u16 {
    p.clamp(10, 70) as u16
}

fn describe(a: &ConfirmAction) -> String {
    match a {
        ConfirmAction::DeleteContainer { name, force, .. } => {
            format!("delete container {} (force={})", name, force)
        }
        ConfirmAction::StopContainer { name, .. } => format!("stop {}", name),
        ConfirmAction::StartContainer { name, .. } => format!("start {}", name),
        ConfirmAction::RestartContainer { name, .. } => format!("restart {}", name),
        ConfirmAction::DeleteImage { label, .. } => format!("delete image {}", label),
        ConfirmAction::DeleteVolume { name, .. } => format!("delete volume {}", name),
        ConfirmAction::DeleteNetwork { name, .. } => format!("delete network {}", name),
        ConfirmAction::PruneImages => "prune dangling images".into(),
        ConfirmAction::PruneVolumes => "prune unused volumes".into(),
        ConfirmAction::PruneNetworks => "prune unused networks".into(),
    }
}

async fn run_action(client: &DockerClient, action: ConfirmAction) -> Result<String> {
    match action {
        ConfirmAction::DeleteContainer { id, force, .. } => {
            containers::remove(client, &id, force, false).await?;
            Ok("removed".into())
        }
        ConfirmAction::StopContainer { id, .. } => {
            containers::stop(client, &id, 10).await?;
            Ok("stopped".into())
        }
        ConfirmAction::StartContainer { id, .. } => {
            containers::start(client, &id).await?;
            Ok("started".into())
        }
        ConfirmAction::RestartContainer { id, .. } => {
            containers::restart(client, &id, 10).await?;
            Ok("restarted".into())
        }
        ConfirmAction::DeleteImage { id, force, .. } => {
            images::remove(client, &id, force).await?;
            Ok("removed".into())
        }
        ConfirmAction::DeleteVolume { name, force, .. } => {
            volumes::remove(client, &name, force).await?;
            Ok("removed".into())
        }
        ConfirmAction::DeleteNetwork { id, .. } => {
            networks::remove(client, &id).await?;
            Ok("removed".into())
        }
        ConfirmAction::PruneImages => {
            let r = images::prune_dangling(client).await?;
            Ok(format!(
                "{} image(s) pruned, {} reclaimed",
                r.deleted,
                images::format_size(r.space_reclaimed),
            ))
        }
        ConfirmAction::PruneVolumes => {
            let r = volumes::prune_unused(client).await?;
            Ok(format!(
                "{} volume(s) pruned, {} reclaimed",
                r.deleted,
                images::format_size(r.space_reclaimed),
            ))
        }
        ConfirmAction::PruneNetworks => {
            let r = networks::prune_unused(client).await?;
            Ok(format!("{} network(s) pruned", r.deleted))
        }
    }
}

/// Where a screen coordinate landed, resolved from the last drawn geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Tab(usize),
    PanelRow { panel: usize, row: usize },
    PanelBody(usize),
    Logs,
    Splitter,
    Nothing,
}

/// Case-insensitive "any field contains" test. `q` must already be lowercased;
/// an empty query matches every row.
fn row_matches(q: &str, fields: [&str; 2]) -> bool {
    if q.is_empty() {
        return true;
    }
    fields.iter().any(|f| f.to_lowercase().contains(q))
}

/// Indices of `rows` whose fields match `q`. This is the single definition of
/// "visible" — `App::visible` caches its output.
fn matching_indices<T>(rows: &[T], q: &str, fields: impl Fn(&T) -> [&str; 2]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, r)| row_matches(q, fields(r)))
        .map(|(i, _)| i)
        .collect()
}

/// Map a cursor position in the filtered view back to an index into the full
/// list. Every selection lookup goes through here, so a stale `visible` shows
/// up as a wrong row rather than an out-of-bounds panic.
fn view_to_row(visible: &[usize], selected: Option<usize>) -> Option<usize> {
    visible.get(selected?).copied()
}

fn contains(r: Rect, col: u16, row: u16) -> bool {
    r.width > 0
        && r.height > 0
        && col >= r.x
        && col < r.x.saturating_add(r.width)
        && row >= r.y
        && row < r.y.saturating_add(r.height)
}

pub async fn run<B>(
    terminal: &mut Terminal<B>,
    client: DockerClient,
    cfg: Config,
    clipboard: Box<dyn Clipboard>,
) -> Result<()>
where
    B: Backend,
    <B as Backend>::Error: Send + Sync + 'static,
{
    let (msg_tx, mut msg_rx) = mpsc::unbounded_channel::<AppMsg>();
    let mut app = App::new(client, cfg.clone(), clipboard, msg_tx);

    // initial load
    let _ = app.refresh_all().await;
    let mut prev_panel = app.panel;

    let mut events = EventStream::new();
    let mut tick = time::interval(Duration::from_millis(cfg.refresh_ms));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    // Hard cap on frame rate: 30 FPS is plenty for a TUI and avoids re-rendering
    // for every individual log line on a chatty container (the main source of CPU).
    const FRAME_BUDGET: Duration = Duration::from_millis(33);
    let mut last_draw = Instant::now() - FRAME_BUDGET;
    let mut dirty = true;

    // Containers change on their own; images/volumes/networks change only on
    // user action, which posts AppMsg::Refresh. Poll the latter every Nth tick.
    const FULL_REFRESH_EVERY: u64 = 5;
    let mut ticks: u64 = 0;

    loop {
        // Draw only if state changed AND the frame budget has elapsed.
        if dirty && last_draw.elapsed() >= FRAME_BUDGET {
            terminal.draw(|f| crate::ui::draw(f, &mut app))?;
            last_draw = Instant::now();
            dirty = false;
        }
        if app.should_quit {
            break;
        }

        // Compute the sleep needed to wake up exactly when the next frame is due.
        let until_next_frame = if dirty {
            FRAME_BUDGET.saturating_sub(last_draw.elapsed())
        } else {
            // No pending work — sleep up to 1s; events/ticks will preempt.
            Duration::from_secs(1)
        };

        tokio::select! {
            biased;
            maybe_evt = events.next() => {
                if let Some(Ok(evt)) = maybe_evt {
                    match evt {
                        Event::Key(key) => app.handle_key(key),
                        Event::Mouse(m) => app.handle_mouse(m),
                        Event::Resize(_, _) => {}
                        _ => {}
                    }
                    if prev_panel != app.panel {
                        prev_panel = app.panel;
                        app.spawn_refresh(true);
                    }
                    dirty = true;
                }
            }
            _ = tick.tick(), if !app.mode_is_confirm() && !app.mode_is_help() => {
                ticks = ticks.wrapping_add(1);
                app.spawn_refresh(ticks.is_multiple_of(FULL_REFRESH_EVERY));
                dirty = true;
            }
            Some(msg) = msg_rx.recv() => {
                app.ingest_msg(msg);
                // Drain everything else in the channel right now so a burst of
                // 200 log lines costs us ONE redraw, not 200.
                while let Ok(more) = msg_rx.try_recv() {
                    app.ingest_msg(more);
                }
                dirty = true;
            }
            _ = tokio::time::sleep(until_next_frame), if dirty => {
                // Wake up to render the pending dirty frame.
            }
        }
    }

    if let Some(h) = app.log_task.take() {
        h.abort();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_cycle() {
        assert_eq!(Panel::Containers.next(), Panel::Images);
        assert_eq!(Panel::Images.next(), Panel::Volumes);
        assert_eq!(Panel::Volumes.next(), Panel::Networks);
        assert_eq!(Panel::Networks.next(), Panel::Containers);
        assert_eq!(Panel::Containers.prev(), Panel::Networks);
    }

    #[test]
    fn panel_index_round_trips() {
        for p in [
            Panel::Containers,
            Panel::Images,
            Panel::Volumes,
            Panel::Networks,
        ] {
            assert!(p.index() < 4);
        }
    }

    #[test]
    fn clamp_split_bounds() {
        assert_eq!(clamp_split(0), 10);
        assert_eq!(clamp_split(-99), 10);
        assert_eq!(clamp_split(35), 35);
        assert_eq!(clamp_split(70), 70);
        assert_eq!(clamp_split(95), 70);
    }

    /// Compute the split row directly (no App needed) — mirrors is_on_splitter
    /// so we can sanity-check the math.
    fn split_row(body_y: u16, body_h: u16, pct: u16) -> u16 {
        body_y + (body_h * pct / 100)
    }

    #[derive(Debug)]
    struct R(&'static str, &'static str);

    fn rows() -> Vec<R> {
        vec![
            R("web-1", "nginx:latest"),
            R("db-1", "postgres:16"),
            R("cache-1", "redis:7"),
            R("web-2", "NGINX:alpine"),
        ]
    }

    #[test]
    fn empty_query_matches_every_row() {
        let idx = matching_indices(&rows(), "", |r| [r.0, r.1]);
        assert_eq!(idx, vec![0, 1, 2, 3]);
    }

    #[test]
    fn filter_matches_either_field_case_insensitively() {
        let r = rows();
        // matches the name field
        assert_eq!(matching_indices(&r, "web", |r| [r.0, r.1]), vec![0, 3]);
        // matches the image field, and upper-case source text still matches
        assert_eq!(matching_indices(&r, "nginx", |r| [r.0, r.1]), vec![0, 3]);
        // second field ignored when the caller does not supply one
        assert_eq!(
            matching_indices(&r, "nginx", |r| [r.0, ""]),
            Vec::<usize>::new()
        );
        assert_eq!(
            matching_indices(&r, "zzz", |r| [r.0, r.1]),
            Vec::<usize>::new()
        );
    }

    /// The invariant the `visible` cache exists to uphold: a cursor position in
    /// the filtered view must resolve to the matching row of the FULL list.
    #[test]
    fn selection_maps_through_filtered_view_to_the_right_row() {
        let r = rows();
        let visible = matching_indices(&r, "web", |r| [r.0, r.1]);
        assert_eq!(visible, vec![0, 3]);
        // cursor 1 in the filtered view is row 3 of the full list, not row 1
        assert_eq!(view_to_row(&visible, Some(1)), Some(3));
        assert_eq!(r[view_to_row(&visible, Some(1)).unwrap()].0, "web-2");
        assert_eq!(view_to_row(&visible, Some(0)), Some(0));
        // past the end of the filtered view, and no selection at all
        assert_eq!(view_to_row(&visible, Some(2)), None);
        assert_eq!(view_to_row(&visible, None), None);
        // an empty view can never resolve
        assert_eq!(view_to_row(&[], Some(0)), None);
    }

    #[test]
    fn contains_is_half_open_and_rejects_empty_rects() {
        let r = Rect {
            x: 2,
            y: 3,
            width: 4,
            height: 2,
        };
        assert!(contains(r, 2, 3));
        assert!(contains(r, 5, 4));
        assert!(!contains(r, 6, 4)); // one past the right edge
        assert!(!contains(r, 5, 5)); // one past the bottom edge
        assert!(!contains(r, 1, 3));
        let empty = Rect {
            x: 2,
            y: 3,
            width: 0,
            height: 2,
        };
        assert!(!contains(empty, 2, 3));
    }

    #[test]
    fn splitter_row_math() {
        // body from y=2, height=20, default split 35% → row 2 + 7 = 9
        assert_eq!(split_row(2, 20, 35), 9);
        assert_eq!(split_row(0, 100, 50), 50);
        assert_eq!(split_row(0, 100, 10), 10);
        assert_eq!(split_row(0, 100, 70), 70);
    }
}
