//! Bounded term counter: when it grows past its limit it keeps the top
//! three quarters (Gonzo's pruning rule), so memory stays flat on long runs.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct FrequencyTable {
    counts: HashMap<String, u64>,
    max_size: usize,
}

impl FrequencyTable {
    pub fn new(max_size: usize) -> Self {
        Self {
            counts: HashMap::new(),
            max_size: max_size.max(1),
        }
    }

    pub fn add(&mut self, term: &str) {
        match self.counts.get_mut(term) {
            Some(count) => *count += 1,
            None => {
                self.counts.insert(term.to_string(), 1);
            }
        }
        if self.counts.len() > self.max_size {
            self.prune();
        }
    }

    pub fn len(&self) -> usize {
        self.counts.len()
    }

    /// Top `n` terms, highest count first, ties alphabetical. Selects the
    /// top `n` first and sorts only those, not the whole table.
    pub fn top(&self, n: usize) -> Vec<(String, u64)> {
        let order =
            |a: &(&String, &u64), b: &(&String, &u64)| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0));
        let mut entries: Vec<(&String, &u64)> = self.counts.iter().collect();
        if n == 0 {
            return Vec::new();
        }
        if n < entries.len() {
            entries.select_nth_unstable_by(n - 1, order);
            entries.truncate(n);
        }
        entries.sort_by(order);
        entries.into_iter().map(|(k, v)| (k.clone(), *v)).collect()
    }

    fn prune(&mut self) {
        let keep = (self.max_size * 3 / 4).max(1);
        self.counts = self.top(keep).into_iter().collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_ranks_terms() {
        let mut t = FrequencyTable::new(100);
        for w in ["b", "a", "b", "c", "b", "a"] {
            t.add(w);
        }
        assert_eq!(t.top(2), vec![("b".to_string(), 3), ("a".to_string(), 2)]);
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn ties_sort_alphabetically() {
        let mut t = FrequencyTable::new(100);
        t.add("zeta");
        t.add("alpha");
        assert_eq!(t.top(5)[0].0, "alpha");
    }

    #[test]
    fn partial_top_matches_a_full_sort() {
        let mut t = FrequencyTable::new(10_000);
        let mut state: u64 = 7;
        for _ in 0..20_000 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            // Skewed distribution with many ties.
            let k = ((state >> 33) % 97) * ((state >> 40) % 7);
            t.add(&format!("w{k}"));
        }
        let mut all: Vec<(String, u64)> = t.counts.iter().map(|(k, v)| (k.clone(), *v)).collect();
        all.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        for n in [0, 1, 3, 10, 50, 100, all.len(), all.len() + 5] {
            let expected: Vec<(String, u64)> = all.iter().take(n).cloned().collect();
            assert_eq!(t.top(n), expected, "n = {n}");
        }
    }

    #[test]
    fn pruning_keeps_the_top_three_quarters() {
        let mut t = FrequencyTable::new(4);
        for (term, times) in [("a", 5), ("b", 4), ("c", 3), ("d", 2)] {
            for _ in 0..times {
                t.add(term);
            }
        }
        t.add("e"); // 5 terms > 4: keep 3
        assert_eq!(t.len(), 3);
        let kept: Vec<String> = t.top(10).into_iter().map(|(k, _)| k).collect();
        assert_eq!(kept, vec!["a", "b", "c"]);
    }
}
