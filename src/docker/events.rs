//! Docker's event stream: which events change a list dox shows, and a
//! watcher that reconnects when the stream drops (e.g. a Colima restart).

use std::collections::HashMap;
use std::time::Duration;

use bollard::query_parameters::EventsOptionsBuilder;
use bollard::Docker;
use futures_util::StreamExt;
use tokio::task::JoinHandle;

/// Which lists an event makes stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stale {
    Containers,
    /// Images, volumes or networks (refreshed together; all but images are cheap).
    Everything,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerSignal {
    Changed(Stale),
    /// The event stream is (or is no longer) delivering; without it dox polls.
    Live(bool),
}

/// The list an event changes, or `None` for events that change nothing dox
/// shows. Healthcheck probes (`exec_*`) fire every few seconds per container
/// and must not trigger refreshes.
pub fn stale_for(kind: &str, action: &str) -> Option<Stale> {
    // "health_status: healthy" and friends carry detail after the colon.
    let verb = action.split(':').next().unwrap_or(action).trim();
    match (kind, verb) {
        (
            "container",
            "start" | "stop" | "die" | "kill" | "create" | "destroy" | "rename" | "pause"
            | "unpause" | "restart" | "oom" | "update" | "health_status",
        ) => Some(Stale::Containers),
        ("image", "pull" | "delete" | "tag" | "untag" | "import" | "load" | "prune")
        | ("volume", "create" | "destroy" | "prune")
        | ("network", "create" | "destroy" | "prune") => Some(Stale::Everything),
        _ => None,
    }
}

/// Longest wait between reconnect attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// A stream that stays open this long without an error counts as live, even
/// if nothing has happened yet (a quiet daemon sends no events).
const CONNECT_GRACE: Duration = Duration::from_secs(2);

/// Follow Docker's event stream until the returned task is aborted, telling
/// `notify` about list changes and whether the stream is live. Reconnects
/// with backoff when the stream ends or fails.
pub fn watch(docker: Docker, notify: impl Fn(DockerSignal) + Send + 'static) -> JoinHandle<()> {
    tokio::spawn(async move {
        let filters = HashMap::from([("type", vec!["container", "image", "volume", "network"])]);
        let mut backoff = Duration::from_secs(1);
        loop {
            let options = EventsOptionsBuilder::default().filters(&filters).build();
            let mut stream = Box::pin(docker.events(Some(options)));
            let mut next = tokio::time::timeout(CONNECT_GRACE, stream.next()).await;
            let connected = match &next {
                Err(_quiet) => true,
                Ok(Some(Ok(_))) => true,
                Ok(Some(Err(_)) | None) => false,
            };
            if connected {
                backoff = Duration::from_secs(1);
                notify(DockerSignal::Live(true));
                loop {
                    let event = match next {
                        Err(_quiet) => None,
                        Ok(Some(Ok(event))) => Some(event),
                        Ok(Some(Err(_)) | None) => break,
                    };
                    if let Some(event) = event {
                        let kind = event.typ.map(|t| t.to_string()).unwrap_or_default();
                        let action = event.action.unwrap_or_default();
                        if let Some(stale) = stale_for(&kind, &action) {
                            notify(DockerSignal::Changed(stale));
                        }
                    }
                    next = Ok(stream.next().await);
                }
                notify(DockerSignal::Live(false));
            }
            tracing::warn!(retry_in = ?backoff, "docker event stream ended; polling until it reconnects");
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_lifecycle_events_refresh_containers() {
        for action in [
            "start", "stop", "die", "kill", "create", "destroy", "rename", "pause", "unpause",
            "restart", "oom", "update",
        ] {
            assert_eq!(
                stale_for("container", action),
                Some(Stale::Containers),
                "{action}"
            );
        }
        assert_eq!(
            stale_for("container", "health_status: healthy"),
            Some(Stale::Containers)
        );
    }

    #[test]
    fn healthcheck_probes_and_terminal_noise_are_ignored() {
        for action in [
            "exec_create: sh -c true",
            "exec_start: sh -c true",
            "exec_die",
            "attach",
            "resize",
            "top",
            "archive-path",
        ] {
            assert_eq!(stale_for("container", action), None, "{action}");
        }
    }

    #[test]
    fn image_volume_network_changes_refresh_everything() {
        assert_eq!(stale_for("image", "pull"), Some(Stale::Everything));
        assert_eq!(stale_for("image", "delete"), Some(Stale::Everything));
        assert_eq!(stale_for("volume", "create"), Some(Stale::Everything));
        assert_eq!(stale_for("network", "destroy"), Some(Stale::Everything));
        assert_eq!(
            stale_for("network", "connect"),
            None,
            "dox doesn't show connections"
        );
        assert_eq!(stale_for("volume", "mount"), None);
        assert_eq!(stale_for("daemon", "reload"), None);
    }
}
