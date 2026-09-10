use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Top-level user actions emitted by the keymap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    NextPanel,
    PrevPanel,
    /// Jump straight to a panel by its index (0-3).
    SelectPanel(usize),
    /// Esc in normal mode: unwind one layer of state (see `App::go_back`).
    Back,
    SelectNext,
    SelectPrev,
    SelectTop,
    SelectBottom,
    PageDown,
    PageUp,
    LogScrollUp,
    LogScrollDown,
    LogJumpTop,
    LogJumpBottom,
    FocusDetail,
    UnfocusDetail,
    StartContainer,
    StopContainer,
    RestartContainer,
    DeleteSelected,
    PruneCurrentPanel,
    ToggleLogs,
    ToggleFollow,
    EnterVisualMode,
    YankSelection,
    BeginFilter,
    AppendFilter(char),
    BackspaceFilter,
    CommitFilter,
    CancelFilter,
    ShowHelp,
    ToggleMouseCapture,
    RestartLogStream,
    GrowPanels,
    ShrinkPanels,
    ResetPanels,
    Confirm,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Filtering,
    Visual,
    Confirm,
    Help,
}

pub fn map(key: KeyEvent, mode: Mode) -> Option<Action> {
    use KeyCode::*;
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            Char('c') => return Some(Action::Quit),
            Char('d') => return Some(Action::PageDown),
            Char('u') => return Some(Action::PageUp),
            Char('f') => return Some(Action::PageDown),
            Char('b') => return Some(Action::PageUp),
            _ => {}
        }
    }

    match mode {
        Mode::Filtering => match key.code {
            Esc => Some(Action::CancelFilter),
            Enter => Some(Action::CommitFilter),
            Backspace => Some(Action::BackspaceFilter),
            Char(c) => Some(Action::AppendFilter(c)),
            _ => None,
        },
        Mode::Confirm => match key.code {
            Char('y') | Char('Y') => Some(Action::Confirm),
            _ => Some(Action::Cancel),
        },
        Mode::Help => match key.code {
            Esc | Char('?') | Char('q') => Some(Action::Cancel),
            _ => None,
        },
        Mode::Visual => match key.code {
            Esc => Some(Action::UnfocusDetail),
            Char('y') => Some(Action::YankSelection),
            Up => Some(Action::SelectPrev),
            Down => Some(Action::SelectNext),
            PageUp | Char('K') => Some(Action::PageUp),
            PageDown | Char('J') => Some(Action::PageDown),
            Char('g') => Some(Action::SelectTop),
            Char('G') => Some(Action::SelectBottom),
            _ => None,
        },
        Mode::Normal => match key.code {
            Char('q') => Some(Action::Quit),
            Tab | Right => Some(Action::NextPanel),
            BackTab | Left => Some(Action::PrevPanel),
            Char(c @ '1'..='4') => Some(Action::SelectPanel(c as usize - '1' as usize)),
            Up => Some(Action::SelectPrev),
            Down => Some(Action::SelectNext),
            PageUp | Char('K') => Some(Action::PageUp),
            PageDown | Char('J') | Char(' ') => Some(Action::PageDown),
            Char('k') => Some(Action::LogScrollUp),
            Char('j') => Some(Action::LogScrollDown),
            Home => Some(Action::LogJumpTop),
            End => Some(Action::LogJumpBottom),
            Char('g') => Some(Action::SelectTop),
            Char('G') => Some(Action::SelectBottom),
            Enter => Some(Action::FocusDetail),
            Esc => Some(Action::Back),
            Char('x') => Some(Action::StopContainer),
            Char('s') => Some(Action::StartContainer),
            Char('r') => Some(Action::RestartContainer),
            Char('d') => Some(Action::DeleteSelected),
            Char('D') => Some(Action::PruneCurrentPanel),
            Char('l') => Some(Action::ToggleLogs),
            Char('f') => Some(Action::ToggleFollow),
            Char('v') => Some(Action::EnterVisualMode),
            Char('y') => Some(Action::YankSelection),
            Char('/') => Some(Action::BeginFilter),
            Char('?') => Some(Action::ShowHelp),
            Char('m') => Some(Action::ToggleMouseCapture),
            Char('R') => Some(Action::RestartLogStream),
            Char('+') => Some(Action::GrowPanels),
            Char('-') => Some(Action::ShrinkPanels),
            Char('=') => Some(Action::ResetPanels),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEventKind, KeyEventState};

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::empty(),
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn k_ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn normal_mode_basic_navigation() {
        assert_eq!(map(k(KeyCode::Char('q')), Mode::Normal), Some(Action::Quit));
        assert_eq!(map(k(KeyCode::Tab), Mode::Normal), Some(Action::NextPanel));
        assert_eq!(
            map(k(KeyCode::BackTab), Mode::Normal),
            Some(Action::PrevPanel)
        );
        assert_eq!(map(k(KeyCode::Up), Mode::Normal), Some(Action::SelectPrev));
        assert_eq!(
            map(k(KeyCode::Down), Mode::Normal),
            Some(Action::SelectNext)
        );
    }

    #[test]
    fn arrows_and_digits_switch_panels() {
        assert_eq!(
            map(k(KeyCode::Right), Mode::Normal),
            Some(Action::NextPanel)
        );
        assert_eq!(map(k(KeyCode::Left), Mode::Normal), Some(Action::PrevPanel));
        for (ch, idx) in [('1', 0), ('2', 1), ('3', 2), ('4', 3)] {
            assert_eq!(
                map(k(KeyCode::Char(ch)), Mode::Normal),
                Some(Action::SelectPanel(idx))
            );
        }
        // '5' is not a panel and must not shadow anything.
        assert_eq!(map(k(KeyCode::Char('5')), Mode::Normal), None);
    }

    #[test]
    fn esc_in_normal_mode_is_back() {
        assert_eq!(map(k(KeyCode::Esc), Mode::Normal), Some(Action::Back));
    }

    #[test]
    fn normal_mode_actions() {
        assert_eq!(
            map(k(KeyCode::Char('x')), Mode::Normal),
            Some(Action::StopContainer)
        );
        assert_eq!(
            map(k(KeyCode::Char('s')), Mode::Normal),
            Some(Action::StartContainer)
        );
        assert_eq!(
            map(k(KeyCode::Char('r')), Mode::Normal),
            Some(Action::RestartContainer)
        );
        assert_eq!(
            map(k(KeyCode::Char('d')), Mode::Normal),
            Some(Action::DeleteSelected)
        );
        assert_eq!(
            map(k(KeyCode::Char('D')), Mode::Normal),
            Some(Action::PruneCurrentPanel)
        );
        assert_eq!(
            map(k(KeyCode::Char('l')), Mode::Normal),
            Some(Action::ToggleLogs)
        );
        assert_eq!(
            map(k(KeyCode::Char('f')), Mode::Normal),
            Some(Action::ToggleFollow)
        );
        assert_eq!(
            map(k(KeyCode::Char('y')), Mode::Normal),
            Some(Action::YankSelection)
        );
    }

    #[test]
    fn ctrl_c_always_quits() {
        assert_eq!(
            map(k_ctrl(KeyCode::Char('c')), Mode::Normal),
            Some(Action::Quit)
        );
        assert_eq!(
            map(k_ctrl(KeyCode::Char('c')), Mode::Filtering),
            Some(Action::Quit)
        );
        assert_eq!(
            map(k_ctrl(KeyCode::Char('c')), Mode::Confirm),
            Some(Action::Quit)
        );
    }

    #[test]
    fn confirm_mode_requires_y() {
        assert_eq!(
            map(k(KeyCode::Char('y')), Mode::Confirm),
            Some(Action::Confirm)
        );
        assert_eq!(
            map(k(KeyCode::Char('n')), Mode::Confirm),
            Some(Action::Cancel)
        );
        assert_eq!(map(k(KeyCode::Enter), Mode::Confirm), Some(Action::Cancel));
        assert_eq!(map(k(KeyCode::Esc), Mode::Confirm), Some(Action::Cancel));
    }

    #[test]
    fn filtering_mode_appends_and_commits() {
        assert_eq!(
            map(k(KeyCode::Char('a')), Mode::Filtering),
            Some(Action::AppendFilter('a'))
        );
        assert_eq!(
            map(k(KeyCode::Backspace), Mode::Filtering),
            Some(Action::BackspaceFilter)
        );
        assert_eq!(
            map(k(KeyCode::Enter), Mode::Filtering),
            Some(Action::CommitFilter)
        );
        assert_eq!(
            map(k(KeyCode::Esc), Mode::Filtering),
            Some(Action::CancelFilter)
        );
    }

    #[test]
    fn visual_mode_yank() {
        assert_eq!(
            map(k(KeyCode::Char('y')), Mode::Visual),
            Some(Action::YankSelection)
        );
        assert_eq!(
            map(k(KeyCode::Esc), Mode::Visual),
            Some(Action::UnfocusDetail)
        );
    }

    #[test]
    fn help_mode_dismisses_on_any_action_key() {
        assert_eq!(map(k(KeyCode::Esc), Mode::Help), Some(Action::Cancel));
        assert_eq!(map(k(KeyCode::Char('?')), Mode::Help), Some(Action::Cancel));
        assert_eq!(map(k(KeyCode::Char('q')), Mode::Help), Some(Action::Cancel));
    }
}
