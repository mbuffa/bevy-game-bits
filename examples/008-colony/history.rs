//! Per-pawn job history: a capped log of completed work, shown in the
//! selection HUD's History panel (`ui::HistoryPanel`). `director::execute_jobs`
//! writes a `JobCompleted` message at each *true* completion (the point where
//! the work actually finishes, not the later `cleanup_jobs` despawn, which
//! also fires for canceled/voided jobs). `Walk` — recreation strolling — is
//! deliberately never written, so it never appears here.

use std::collections::VecDeque;

use bevy::prelude::*;

use crate::config::MAX_JOB_HISTORY;
use crate::daynight::GameClock;

/// One completed-job row: a timestamp (`GameClock::clock_label`, e.g.
/// `"Day 2 — 14:30"`) plus the human-readable action.
#[derive(Clone)]
pub struct HistoryEntry {
    pub time: String,
    pub label: String,
}

/// A pawn's completed-job log, newest first, capped at `MAX_JOB_HISTORY`.
#[derive(Component, Default)]
pub struct JobHistory(pub VecDeque<HistoryEntry>);

impl JobHistory {
    /// Push a new entry to the front and drop the oldest past the cap.
    fn record(&mut self, entry: HistoryEntry) {
        self.0.push_front(entry);
        self.0.truncate(MAX_JOB_HISTORY);
    }
}

/// A pawn finished a piece of work — written at the exact site in
/// `execute_jobs` where each job kind's action actually completes (not at
/// `cleanup_jobs`'s despawn, which also fires for canceled/voided jobs).
#[derive(Message)]
pub struct JobCompleted {
    pub pawn: Entity,
    pub label: String,
}

/// Append each `JobCompleted` message to its pawn's `JobHistory`, stamped
/// with the current in-game time.
pub fn record_history(
    mut events: MessageReader<JobCompleted>,
    clock: Res<GameClock>,
    mut histories: Query<&mut JobHistory>,
) {
    for event in events.read() {
        if let Ok(mut history) = histories.get_mut(event.pawn) {
            history.record(HistoryEntry {
                time: clock.clock_label(),
                label: event.label.clone(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str) -> HistoryEntry {
        HistoryEntry {
            time: "Day 1 — 08:00".to_string(),
            label: label.to_string(),
        }
    }

    #[test]
    fn newest_entry_goes_first() {
        let mut history = JobHistory::default();
        history.record(entry("first"));
        history.record(entry("second"));
        let labels: Vec<&str> = history.0.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, vec!["second", "first"]);
    }

    #[test]
    fn caps_at_max_job_history() {
        let mut history = JobHistory::default();
        for i in 0..(MAX_JOB_HISTORY + 10) {
            history.record(entry(&i.to_string()));
        }
        assert_eq!(history.0.len(), MAX_JOB_HISTORY);
        // The most recent push is still at the front.
        assert_eq!(
            history.0.front().unwrap().label,
            (MAX_JOB_HISTORY + 9).to_string()
        );
    }
}
