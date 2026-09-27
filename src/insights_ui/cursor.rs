//! Selection-cursor arithmetic for the log list and the insight boxes.

/// Move a cursor over `len` rows by `delta`, clamped. A missing cursor
/// starts from `start` (the row the user is looking at).
pub fn step(cursor: Option<usize>, start: usize, delta: isize, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let last = len - 1;
    let base = cursor.unwrap_or(start).min(last) as isize;
    Some((base + delta).clamp(0, last as isize) as usize)
}

/// Scroll offset that keeps `cursor` inside a window of `height` rows.
pub fn keep_visible(cursor: usize, scroll: usize, height: usize) -> usize {
    if height == 0 {
        return scroll;
    }
    if cursor < scroll {
        cursor
    } else if cursor >= scroll + height {
        cursor + 1 - height
    } else {
        scroll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_starts_from_the_visible_row_and_clamps() {
        assert_eq!(step(None, 9, -1, 10), Some(8));
        assert_eq!(step(Some(0), 9, -1, 10), Some(0));
        assert_eq!(step(Some(8), 9, 5, 10), Some(9));
        assert_eq!(step(None, 0, 1, 0), None);
        assert_eq!(step(None, 50, 0, 10), Some(9));
    }

    #[test]
    fn keep_visible_scrolls_just_enough() {
        assert_eq!(keep_visible(5, 0, 10), 0);
        assert_eq!(keep_visible(12, 0, 10), 3);
        assert_eq!(keep_visible(2, 5, 10), 2);
        assert_eq!(keep_visible(0, 0, 0), 0);
    }
}
