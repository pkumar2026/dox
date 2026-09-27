//! Field (attribute) statistics for the Top Attributes box: how often each key
//! appears and how many distinct values it has, sorted like Gonzo (most
//! distinct values first).

use std::collections::HashMap;

use super::frequency::FrequencyTable;

#[derive(Debug, Clone, PartialEq)]
pub struct AttributeSummary {
    pub key: String,
    pub total: u64,
    pub unique: usize,
}

#[derive(Debug, Clone)]
pub struct AttributeTable {
    keys: HashMap<String, (u64, FrequencyTable)>,
    max_keys: usize,
    max_values: usize,
}

impl AttributeTable {
    pub fn new(max_keys: usize, max_values: usize) -> Self {
        Self {
            keys: HashMap::new(),
            max_keys: max_keys.max(1),
            max_values: max_values.max(1),
        }
    }

    pub fn add(&mut self, attributes: &[(String, String)]) {
        for (key, value) in attributes {
            // Look up by `&str` first: the key is only copied when it is new.
            if let Some((total, values)) = self.keys.get_mut(key.as_str()) {
                *total += 1;
                values.add(value);
                continue;
            }
            let mut values = FrequencyTable::new(self.max_values);
            values.add(value);
            self.keys.insert(key.clone(), (1, values));
        }
        if self.keys.len() > self.max_keys {
            self.prune();
        }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Keys with the most distinct values first; ties by key.
    pub fn top(&self, n: usize) -> Vec<AttributeSummary> {
        let mut all: Vec<AttributeSummary> = self
            .keys
            .iter()
            .map(|(key, (total, values))| AttributeSummary {
                key: key.clone(),
                total: *total,
                unique: values.len(),
            })
            .collect();
        all.sort_by(|a, b| b.unique.cmp(&a.unique).then_with(|| a.key.cmp(&b.key)));
        all.truncate(n);
        all
    }

    /// Most common values of one key.
    pub fn values(&self, key: &str, n: usize) -> Vec<(String, u64)> {
        self.keys
            .get(key)
            .map(|(_, values)| values.top(n))
            .unwrap_or_default()
    }

    /// Keep the three quarters of keys seen most often.
    fn prune(&mut self) {
        let keep = (self.max_keys * 3 / 4).max(1);
        let mut by_total: Vec<(String, u64)> = self
            .keys
            .iter()
            .map(|(k, (total, _))| (k.clone(), *total))
            .collect();
        by_total.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let dropped: Vec<String> = by_total.into_iter().skip(keep).map(|(k, _)| k).collect();
        for key in dropped {
            self.keys.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn sorts_by_distinct_values_then_key() {
        let mut t = AttributeTable::new(100, 100);
        t.add(&pairs(&[("method", "GET"), ("status", "200")]));
        t.add(&pairs(&[("method", "GET"), ("status", "500")]));
        t.add(&pairs(&[("method", "GET"), ("status", "404")]));
        let top = t.top(5);
        assert_eq!(
            top[0],
            AttributeSummary {
                key: "status".into(),
                total: 3,
                unique: 3
            }
        );
        assert_eq!(
            top[1],
            AttributeSummary {
                key: "method".into(),
                total: 3,
                unique: 1
            }
        );
    }

    #[test]
    fn values_for_a_key() {
        let mut t = AttributeTable::new(100, 100);
        t.add(&pairs(&[("status", "200")]));
        t.add(&pairs(&[("status", "200")]));
        t.add(&pairs(&[("status", "500")]));
        assert_eq!(
            t.values("status", 5),
            vec![("200".to_string(), 2), ("500".to_string(), 1)]
        );
        assert!(t.values("missing", 5).is_empty());
    }

    #[test]
    fn distinct_values_per_key_are_capped() {
        let mut t = AttributeTable::new(100, 4);
        for i in 0..50 {
            t.add(&pairs(&[("client", &format!("10.0.0.{i}"))]));
        }
        assert!(t.top(1)[0].unique <= 4);
        assert_eq!(t.top(1)[0].total, 50);
    }

    #[test]
    fn number_of_keys_is_capped() {
        let mut t = AttributeTable::new(4, 10);
        for i in 0..20 {
            t.add(&pairs(&[(&format!("k{i}"), "v")]));
        }
        assert!(t.len() <= 4);
    }
}
