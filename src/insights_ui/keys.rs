//! Keys while the log/insights area is focused, following Gonzo's key table.
//! dox's own keys still apply everywhere else; here only its safe ones pass
//! through (`q`, `?`, `m`, `v`, `y`, Esc) and its container actions are ignored.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::ModalKind;

/// What a key does in the focused log/insights area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsightsKey {
    NextSection,
    PrevSection,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    Left,
    Right,
    /// Section-specific: detail popup, highlight word, values, patterns, counts.
    Open,
    TogglePause,
    BeginFilter,
    BeginSearch,
    SeverityFilter,
    Fullscreen,
    Columns,
    Reset,
    SlowerRefresh,
    FasterRefresh,
    Stats,
    /// Inside a popup: close it (Esc).
    CloseModal,
    /// Inside the severity / column popups: flip the highlighted entry.
    ToggleItem,
    /// Inside the severity / column popups: apply and close.
    Apply,
    /// Log detail popup: copy the message / the whole entry.
    CopyMessage,
    CopyEntry,
    /// Typing a filter or search.
    InputChar(char),
    InputBackspace,
    InputCommit,
    InputCancel,
    /// Hand the key to dox's normal handling.
    Dox(DoxKey),
    Ignore,
}

/// dox keys that keep working in the focused area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoxKey {
    Quit,
    RestartStream,
    GrowPanels,
    ShrinkPanels,
    ResetPanels,
    Help,
    ToggleMouse,
    Visual,
    Yank,
    Unfocus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyContext {
    pub typing: bool,
    pub modal: Option<ModalKind>,
}

pub fn route(key: &KeyEvent, ctx: &KeyContext) -> InsightsKey {
    use KeyCode::*;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && key.code == Char('c') {
        return InsightsKey::Dox(DoxKey::Quit);
    }
    if ctx.typing {
        return match key.code {
            Enter => InsightsKey::InputCommit,
            Esc => InsightsKey::InputCancel,
            Backspace => InsightsKey::InputBackspace,
            Char(c) if !ctrl => InsightsKey::InputChar(c),
            _ => InsightsKey::Ignore,
        };
    }
    match ctx.modal {
        Some(ModalKind::Fullscreen) => fullscreen(key),
        Some(modal) => in_modal(key, modal),
        None => main_view(key, ctrl),
    }
}

/// Up/down/page keys, shared by every view.
fn navigation(code: KeyCode) -> Option<InsightsKey> {
    use KeyCode::*;
    Some(match code {
        Up | Char('k') => InsightsKey::Up,
        Down | Char('j') => InsightsKey::Down,
        PageUp | Char('K') => InsightsKey::PageUp,
        PageDown | Char('J') => InsightsKey::PageDown,
        Home | Char('g') => InsightsKey::Home,
        End | Char('G') => InsightsKey::End,
        _ => return None,
    })
}

fn main_view(key: &KeyEvent, ctrl: bool) -> InsightsKey {
    use KeyCode::*;
    if ctrl {
        return match key.code {
            Char('f') => InsightsKey::SeverityFilter,
            Char('d') => InsightsKey::PageDown,
            Char('u') | Char('b') => InsightsKey::PageUp,
            _ => InsightsKey::Ignore,
        };
    }
    if let Some(nav) = navigation(key.code) {
        return nav;
    }
    match key.code {
        Tab => InsightsKey::NextSection,
        BackTab => InsightsKey::PrevSection,
        Left | Char('h') => InsightsKey::Left,
        Right | Char('l') => InsightsKey::Right,
        Enter => InsightsKey::Open,
        Char(' ') => InsightsKey::TogglePause,
        Char('/') => InsightsKey::BeginFilter,
        Char('s') => InsightsKey::BeginSearch,
        Char('f') => InsightsKey::Fullscreen,
        Char('C') => InsightsKey::Columns,
        Char('r') => InsightsKey::Reset,
        Char('u') => InsightsKey::SlowerRefresh,
        Char('U') => InsightsKey::FasterRefresh,
        Char('i') => InsightsKey::Stats,
        Char('q') => InsightsKey::Dox(DoxKey::Quit),
        Char('?') => InsightsKey::Dox(DoxKey::Help),
        Char('m') => InsightsKey::Dox(DoxKey::ToggleMouse),
        Char('v') => InsightsKey::Dox(DoxKey::Visual),
        Char('y') => InsightsKey::Dox(DoxKey::Yank),
        Char('R') => InsightsKey::Dox(DoxKey::RestartStream),
        Char('+') => InsightsKey::Dox(DoxKey::GrowPanels),
        Char('-') => InsightsKey::Dox(DoxKey::ShrinkPanels),
        Char('=') => InsightsKey::Dox(DoxKey::ResetPanels),
        Esc => InsightsKey::Dox(DoxKey::Unfocus),
        _ => InsightsKey::Ignore,
    }
}

fn in_modal(key: &KeyEvent, modal: ModalKind) -> InsightsKey {
    use KeyCode::*;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if key.code == Esc {
        return InsightsKey::CloseModal;
    }
    if let Some(nav) = navigation(key.code) {
        return nav;
    }
    let toggles = matches!(modal, ModalKind::SeverityFilter | ModalKind::Columns);
    match key.code {
        Char(' ') if toggles => InsightsKey::ToggleItem,
        Enter if toggles => InsightsKey::Apply,
        Char('y') if ctrl && modal == ModalKind::Detail => InsightsKey::CopyEntry,
        Char('y') if modal == ModalKind::Detail => InsightsKey::CopyMessage,
        _ => InsightsKey::Ignore,
    }
}

fn fullscreen(key: &KeyEvent) -> InsightsKey {
    use KeyCode::*;
    if let Some(nav) = navigation(key.code) {
        return nav;
    }
    match key.code {
        Esc | Char('f') => InsightsKey::CloseModal,
        Enter => InsightsKey::Open,
        Left | Char('h') => InsightsKey::Left,
        Right | Char('l') => InsightsKey::Right,
        _ => InsightsKey::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn main() -> KeyContext {
        KeyContext {
            typing: false,
            modal: None,
        }
    }

    fn with_modal(m: ModalKind) -> KeyContext {
        KeyContext {
            modal: Some(m),
            ..main()
        }
    }

    #[test]
    fn gonzo_keys_in_the_main_view() {
        let c = main();
        assert_eq!(route(&k(KeyCode::Tab), &c), InsightsKey::NextSection);
        assert_eq!(route(&k(KeyCode::BackTab), &c), InsightsKey::PrevSection);
        assert_eq!(route(&k(KeyCode::Char('j')), &c), InsightsKey::Down);
        assert_eq!(route(&k(KeyCode::Up), &c), InsightsKey::Up);
        assert_eq!(route(&k(KeyCode::Char('h')), &c), InsightsKey::Left);
        assert_eq!(route(&k(KeyCode::Char('l')), &c), InsightsKey::Right);
        assert_eq!(route(&k(KeyCode::Enter), &c), InsightsKey::Open);
        assert_eq!(route(&k(KeyCode::Char(' ')), &c), InsightsKey::TogglePause);
        assert_eq!(route(&k(KeyCode::Char('/')), &c), InsightsKey::BeginFilter);
        assert_eq!(route(&k(KeyCode::Char('s')), &c), InsightsKey::BeginSearch);
        assert_eq!(route(&ctrl('f'), &c), InsightsKey::SeverityFilter);
        assert_eq!(route(&k(KeyCode::Char('f')), &c), InsightsKey::Fullscreen);
        assert_eq!(route(&k(KeyCode::Char('C')), &c), InsightsKey::Columns);
        assert_eq!(route(&k(KeyCode::Char('r')), &c), InsightsKey::Reset);
        assert_eq!(
            route(&k(KeyCode::Char('u')), &c),
            InsightsKey::SlowerRefresh
        );
        assert_eq!(
            route(&k(KeyCode::Char('U')), &c),
            InsightsKey::FasterRefresh
        );
        assert_eq!(route(&k(KeyCode::Char('i')), &c), InsightsKey::Stats);
        assert_eq!(route(&k(KeyCode::End), &c), InsightsKey::End);
    }

    #[test]
    fn safe_dox_keys_pass_through() {
        let c = main();
        assert_eq!(
            route(&k(KeyCode::Char('q')), &c),
            InsightsKey::Dox(DoxKey::Quit)
        );
        assert_eq!(route(&ctrl('c'), &c), InsightsKey::Dox(DoxKey::Quit));
        assert_eq!(
            route(&k(KeyCode::Char('?')), &c),
            InsightsKey::Dox(DoxKey::Help)
        );
        assert_eq!(
            route(&k(KeyCode::Char('m')), &c),
            InsightsKey::Dox(DoxKey::ToggleMouse)
        );
        assert_eq!(
            route(&k(KeyCode::Char('v')), &c),
            InsightsKey::Dox(DoxKey::Visual)
        );
        assert_eq!(
            route(&k(KeyCode::Char('y')), &c),
            InsightsKey::Dox(DoxKey::Yank)
        );
        assert_eq!(
            route(&k(KeyCode::Esc), &c),
            InsightsKey::Dox(DoxKey::Unfocus)
        );
    }

    #[test]
    fn non_conflicting_dox_keys_keep_working() {
        let c = main();
        assert_eq!(
            route(&k(KeyCode::Char('R')), &c),
            InsightsKey::Dox(DoxKey::RestartStream)
        );
        assert_eq!(
            route(&k(KeyCode::Char('+')), &c),
            InsightsKey::Dox(DoxKey::GrowPanels)
        );
        assert_eq!(
            route(&k(KeyCode::Char('-')), &c),
            InsightsKey::Dox(DoxKey::ShrinkPanels)
        );
        assert_eq!(
            route(&k(KeyCode::Char('=')), &c),
            InsightsKey::Dox(DoxKey::ResetPanels)
        );
        assert_eq!(route(&k(KeyCode::Char('g')), &c), InsightsKey::Home);
        assert_eq!(route(&k(KeyCode::Char('G')), &c), InsightsKey::End);
        assert_eq!(route(&k(KeyCode::Char('K')), &c), InsightsKey::PageUp);
        assert_eq!(route(&k(KeyCode::Char('J')), &c), InsightsKey::PageDown);
        assert_eq!(route(&ctrl('d'), &c), InsightsKey::PageDown);
        assert_eq!(route(&ctrl('u'), &c), InsightsKey::PageUp);
        assert_eq!(route(&ctrl('b'), &c), InsightsKey::PageUp);
    }

    #[test]
    fn container_actions_are_ignored_while_focused() {
        let c = main();
        for ch in ['x', 'd', 'D', 'z'] {
            assert_eq!(
                route(&k(KeyCode::Char(ch)), &c),
                InsightsKey::Ignore,
                "{ch}"
            );
        }
    }

    #[test]
    fn typing_takes_every_key() {
        let c = KeyContext {
            typing: true,
            ..main()
        };
        assert_eq!(
            route(&k(KeyCode::Char('q')), &c),
            InsightsKey::InputChar('q')
        );
        assert_eq!(
            route(&k(KeyCode::Char('/')), &c),
            InsightsKey::InputChar('/')
        );
        assert_eq!(
            route(&k(KeyCode::Backspace), &c),
            InsightsKey::InputBackspace
        );
        assert_eq!(route(&k(KeyCode::Enter), &c), InsightsKey::InputCommit);
        assert_eq!(route(&k(KeyCode::Esc), &c), InsightsKey::InputCancel);
        assert_eq!(route(&ctrl('c'), &c), InsightsKey::Dox(DoxKey::Quit));
    }

    #[test]
    fn popups_close_on_esc_and_scroll() {
        for m in [
            ModalKind::Detail,
            ModalKind::Stats,
            ModalKind::Counts,
            ModalKind::Patterns,
        ] {
            let c = with_modal(m);
            assert_eq!(route(&k(KeyCode::Esc), &c), InsightsKey::CloseModal);
            assert_eq!(route(&k(KeyCode::Down), &c), InsightsKey::Down);
            assert_eq!(route(&k(KeyCode::PageDown), &c), InsightsKey::PageDown);
            assert_eq!(route(&k(KeyCode::Char('x')), &c), InsightsKey::Ignore);
        }
    }

    #[test]
    fn detail_popup_copies() {
        let c = with_modal(ModalKind::Detail);
        assert_eq!(route(&k(KeyCode::Char('y')), &c), InsightsKey::CopyMessage);
        assert_eq!(route(&ctrl('y'), &c), InsightsKey::CopyEntry);
    }

    #[test]
    fn severity_and_column_popups_toggle_and_apply() {
        for m in [ModalKind::SeverityFilter, ModalKind::Columns] {
            let c = with_modal(m);
            assert_eq!(route(&k(KeyCode::Char(' ')), &c), InsightsKey::ToggleItem);
            assert_eq!(route(&k(KeyCode::Enter), &c), InsightsKey::Apply);
            assert_eq!(route(&k(KeyCode::Char('k')), &c), InsightsKey::Up);
        }
    }

    #[test]
    fn fullscreen_viewer_navigates_logs() {
        let c = with_modal(ModalKind::Fullscreen);
        assert_eq!(route(&k(KeyCode::Down), &c), InsightsKey::Down);
        assert_eq!(route(&k(KeyCode::Enter), &c), InsightsKey::Open);
        assert_eq!(route(&k(KeyCode::Home), &c), InsightsKey::Home);
        assert_eq!(route(&k(KeyCode::Char('f')), &c), InsightsKey::CloseModal);
        assert_eq!(route(&k(KeyCode::Esc), &c), InsightsKey::CloseModal);
    }
}
