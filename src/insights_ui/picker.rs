//! Checkbox list used by the severity filter (Ctrl+f) and column picker (C)
//! popups. Changes apply on Enter; Esc discards the draft.

use crate::insights::severity::Severity;

/// Rows above the severities in the severity popup.
pub const SELECT_ALL: usize = 0;
pub const SELECT_NONE: usize = 1;
const SEVERITY_FIRST_ROW: usize = 2;

#[derive(Debug, Clone, PartialEq)]
pub struct Picker {
    pub cursor: usize,
    /// `(label, checked)`. The first `actions` rows are commands, not options.
    pub items: Vec<(String, bool)>,
    pub actions: usize,
}

impl Picker {
    pub fn move_by(&mut self, delta: isize) {
        let last = self.items.len().saturating_sub(1) as isize;
        self.cursor = (self.cursor as isize + delta).clamp(0, last) as usize;
    }

    pub fn toggle(&mut self) {
        if self.cursor < self.actions {
            return;
        }
        if let Some(item) = self.items.get_mut(self.cursor) {
            item.1 = !item.1;
        }
    }

    pub fn active(&self) -> usize {
        self.items.iter().filter(|(_, on)| *on).count()
    }
}

/// Severity popup for the current `hidden` flags.
pub fn severity_picker(hidden: &[bool; 6]) -> Picker {
    let actions = [
        ("Select All".to_string(), false),
        ("Select None".to_string(), false),
    ];
    let levels = Severity::ALL
        .iter()
        .map(|s| (s.label().to_string(), !hidden[s.index()]));
    Picker {
        cursor: SEVERITY_FIRST_ROW,
        items: actions.into_iter().chain(levels).collect(),
        actions: SEVERITY_FIRST_ROW,
    }
}

/// What Enter does in the severity popup: `hidden` flags to apply.
pub fn apply_severity(picker: &Picker) -> [bool; 6] {
    match picker.cursor {
        SELECT_ALL => [false; 6],
        SELECT_NONE => [true; 6],
        _ => std::array::from_fn(|i| {
            !picker
                .items
                .get(SEVERITY_FIRST_ROW + i)
                .is_some_and(|(_, on)| *on)
        }),
    }
}

/// Log columns (Gonzo's column picker): fixed ones plus discovered fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Columns {
    pub time: bool,
    pub level: bool,
    pub service: bool,
    pub host: bool,
    pub fields: Vec<String>,
}

impl Default for Columns {
    /// dox's usual view: the timestamp, then the line.
    fn default() -> Self {
        Self {
            time: true,
            level: false,
            service: false,
            host: false,
            fields: Vec::new(),
        }
    }
}

impl Columns {
    /// True when rows can use dox's plain rendering.
    pub fn is_default(&self) -> bool {
        *self == Columns::default()
    }
}

const FIXED_COLUMNS: [&str; 4] = ["Time", "Level", "Service", "Host"];

/// Column popup: fixed columns, then discovered field keys.
pub fn column_picker(current: &Columns, discovered: &[String]) -> Picker {
    let fixed = [current.time, current.level, current.service, current.host];
    let mut items: Vec<(String, bool)> = FIXED_COLUMNS
        .iter()
        .zip(fixed)
        .map(|(label, on)| (label.to_string(), on))
        .collect();
    let chosen = |key: &String| current.fields.contains(key);
    items.extend(discovered.iter().map(|k| (k.clone(), chosen(k))));
    items.extend(
        current
            .fields
            .iter()
            .filter(|k| !discovered.contains(k))
            .map(|k| (k.clone(), true)),
    );
    Picker {
        cursor: 0,
        items,
        actions: 0,
    }
}

pub fn apply_columns(picker: &Picker) -> Columns {
    let on = |i: usize| picker.items.get(i).is_some_and(|(_, on)| *on);
    Columns {
        time: on(0),
        level: on(1),
        service: on(2),
        host: on(3),
        fields: picker
            .items
            .iter()
            .skip(FIXED_COLUMNS.len())
            .filter(|(_, on)| *on)
            .map(|(k, _)| k.clone())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_picker_reflects_hidden_levels() {
        let mut hidden = [false; 6];
        hidden[Severity::Debug.index()] = true;
        let p = severity_picker(&hidden);
        assert_eq!(p.items.len(), 8);
        assert_eq!(p.items[SELECT_ALL].0, "Select All");
        assert_eq!(p.items[SEVERITY_FIRST_ROW].0, "FATAL");
        assert!(!p.items[SEVERITY_FIRST_ROW + Severity::Debug.index()].1);
        assert_eq!(p.active(), 5);
    }

    #[test]
    fn toggling_a_severity_and_applying() {
        let mut p = severity_picker(&[false; 6]);
        assert_eq!(p.cursor, SEVERITY_FIRST_ROW, "cursor starts on FATAL");
        p.move_by(Severity::Info.index() as isize);
        p.toggle();
        let hidden = apply_severity(&p);
        assert!(hidden[Severity::Info.index()]);
        assert!(!hidden[Severity::Error.index()]);
    }

    #[test]
    fn select_all_and_none_rows() {
        let mut hidden = [true; 6];
        hidden[0] = false;
        let mut p = severity_picker(&hidden);
        p.cursor = SELECT_ALL;
        assert_eq!(apply_severity(&p), [false; 6]);
        p.cursor = SELECT_NONE;
        assert_eq!(apply_severity(&p), [true; 6]);
    }

    #[test]
    fn cursor_stays_in_range() {
        let mut p = severity_picker(&[false; 6]);
        p.move_by(-5);
        assert_eq!(p.cursor, 0);
        p.move_by(100);
        assert_eq!(p.cursor, 7);
    }

    #[test]
    fn column_picker_round_trip() {
        let discovered = vec!["status_code".to_string(), "method".to_string()];
        let mut p = column_picker(&Columns::default(), &discovered);
        assert_eq!(p.items.len(), 6);
        assert_eq!(p.active(), 1); // Time
        p.cursor = 1; // Level
        p.toggle();
        p.cursor = 4; // status_code
        p.toggle();
        let cols = apply_columns(&p);
        assert!(cols.time && cols.level && !cols.service && !cols.host);
        assert_eq!(cols.fields, vec!["status_code".to_string()]);
        assert!(!cols.is_default());
    }

    #[test]
    fn chosen_fields_stay_listed_even_if_no_longer_discovered() {
        let current = Columns {
            fields: vec!["old_key".into()],
            ..Columns::default()
        };
        let p = column_picker(&current, &[]);
        assert!(p.items.iter().any(|(l, on)| l == "old_key" && *on));
    }
}
