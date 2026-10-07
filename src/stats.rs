//! One stats stream per running container, and the last couple of minutes of
//! samples from each. Streams start and stop as the container list changes; a
//! stopped container's samples go away with its stream.

use std::collections::{HashMap, HashSet, VecDeque};

use tokio::task::JoinHandle;

use crate::docker::stats::ContainerStats;

/// Samples kept per container for the `o` graphs. Streams send about one a
/// second, so this is roughly the last two minutes.
pub const HISTORY_LEN: usize = 120;

#[derive(Debug, Default)]
pub struct StatsStreams {
    tasks: HashMap<String, JoinHandle<()>>,
    history: HashMap<String, VecDeque<ContainerStats>>,
}

impl StatsStreams {
    /// Match the open streams to `running`: stop those whose container is gone
    /// or stopped, and `start` any that are missing or ended on their own.
    pub fn sync(
        &mut self,
        running: &HashSet<String>,
        mut start: impl FnMut(&str) -> JoinHandle<()>,
    ) {
        self.tasks.retain(|id, task| {
            let keep = running.contains(id) && !task.is_finished();
            if !keep {
                task.abort();
            }
            keep
        });
        self.history.retain(|id, _| running.contains(id));
        for id in running {
            self.tasks.entry(id.clone()).or_insert_with(|| start(id));
        }
    }

    /// A sample arrived. Dropped once its stream was stopped, so a message
    /// still in flight can't bring back a stopped container's numbers.
    pub fn record(&mut self, id: String, stats: ContainerStats) {
        if !self.tasks.contains_key(&id) {
            return;
        }
        let samples = self.history.entry(id).or_default();
        if samples.len() == HISTORY_LEN {
            samples.pop_front();
        }
        samples.push_back(stats);
    }

    /// The newest sample.
    pub fn get(&self, id: &str) -> Option<&ContainerStats> {
        self.history.get(id)?.back()
    }

    /// Oldest first, at most `HISTORY_LEN`.
    pub fn history(&self, id: &str) -> Option<&VecDeque<ContainerStats>> {
        self.history.get(id)
    }

    pub fn stop_all(&mut self) {
        for (_, task) in self.tasks.drain() {
            task.abort();
        }
        self.history.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn sample(mem_used: u64) -> ContainerStats {
        ContainerStats {
            mem_used,
            ..ContainerStats::default()
        }
    }

    fn pending_task(started: &mut Vec<String>, id: &str) -> JoinHandle<()> {
        started.push(id.to_string());
        tokio::spawn(std::future::pending())
    }

    #[tokio::test]
    async fn opens_one_stream_per_running_container() {
        let mut streams = StatsStreams::default();
        let mut started = Vec::new();
        streams.sync(&ids(&["a", "b"]), |id| pending_task(&mut started, id));
        streams.sync(&ids(&["a", "b"]), |id| pending_task(&mut started, id));
        started.sort();
        assert_eq!(started, vec!["a", "b"], "a second sync opens nothing new");
    }

    #[tokio::test]
    async fn stopping_a_container_closes_its_stream_and_drops_its_numbers() {
        let mut streams = StatsStreams::default();
        let mut started = Vec::new();
        streams.sync(&ids(&["a", "b"]), |id| pending_task(&mut started, id));
        streams.record("a".into(), sample(1));
        streams.record("b".into(), sample(2));

        streams.sync(&ids(&["a"]), |id| pending_task(&mut started, id));

        assert!(!streams.tasks.contains_key("b"));
        assert_eq!(streams.get("b"), None);
        assert_eq!(streams.get("a"), Some(&sample(1)));
    }

    #[tokio::test]
    async fn a_stream_that_ended_is_reopened_while_the_container_runs() {
        let mut streams = StatsStreams::default();
        let mut started = Vec::new();
        streams.sync(&ids(&["a"]), |id| {
            started.push(id.to_string());
            tokio::spawn(async {})
        });
        tokio::task::yield_now().await;
        while !streams.tasks["a"].is_finished() {
            tokio::task::yield_now().await;
        }
        streams.sync(&ids(&["a"]), |id| pending_task(&mut started, id));
        assert_eq!(started, vec!["a", "a"]);
    }

    #[tokio::test]
    async fn late_samples_for_a_stopped_stream_are_ignored() {
        let mut streams = StatsStreams::default();
        let mut started = Vec::new();
        streams.sync(&ids(&["a"]), |id| pending_task(&mut started, id));
        streams.sync(&ids(&[]), |id| pending_task(&mut started, id));
        streams.record("a".into(), sample(1));
        assert_eq!(streams.get("a"), None);
    }

    #[tokio::test]
    async fn keeps_the_last_two_minutes_of_samples() {
        let mut streams = StatsStreams::default();
        let mut started = Vec::new();
        streams.sync(&ids(&["a"]), |id| pending_task(&mut started, id));
        for n in 0..(HISTORY_LEN as u64 + 5) {
            streams.record("a".into(), sample(n));
        }
        let history = streams.history("a").expect("a has samples");
        assert_eq!(history.len(), HISTORY_LEN);
        assert_eq!(history.front().map(|s| s.mem_used), Some(5));
        assert_eq!(
            streams.get("a").map(|s| s.mem_used),
            Some(HISTORY_LEN as u64 + 4),
            "get returns the newest sample"
        );
    }

    #[tokio::test]
    async fn stop_all_closes_every_stream() {
        let mut streams = StatsStreams::default();
        let mut started = Vec::new();
        streams.sync(&ids(&["a", "b"]), |id| pending_task(&mut started, id));
        streams.record("a".into(), sample(1));
        streams.stop_all();
        assert!(streams.tasks.is_empty());
        assert_eq!(streams.get("a"), None);
    }
}
