//! Bounded per-connection chat and recall retry bookkeeping.
use crate::model::Message;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use uuid::Uuid;

pub(crate) const RETRANSMIT_TICK: Duration = Duration::from_secs(4);
pub(crate) const PER_TICK: usize = 2;
pub(crate) const BATCH: usize = 32;
const MAX_TRACKED: usize = 512;
const MAX_RECEIPTS: usize = 64;

#[derive(Default)]
pub(crate) struct SessionState {
    pub cursor: i64,
    pub queued: HashSet<String>,
    sent: HashMap<String, (Instant, u32)>,
    receipts: VecDeque<Uuid>,
    retries: VecDeque<(String, Message)>,
    pub receipt_ready: Arc<Notify>,
}

impl SessionState {
    pub fn due(&self, id: &str, now: Instant) -> bool {
        if self.queued.contains(id) {
            return false;
        }
        self.sent.get(id).map_or(true, |(at, count)| {
            now.saturating_duration_since(*at) >= Duration::from_secs(6 * (1u64 << (*count).min(3)))
        })
    }

    // Called once, after an actual socket write. No permanent retry cutoff.
    pub fn record_sent(&mut self, id: &str, now: Instant) {
        self.queued.remove(id);
        let count = self
            .sent
            .get(id)
            .map_or(0, |(_, count)| count.saturating_add(1));
        if self.sent.len() >= MAX_TRACKED && !self.sent.contains_key(id) {
            if let Some(oldest) = self
                .sent
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(id, _)| id.clone())
            {
                self.sent.remove(&oldest);
            }
        }
        self.sent.insert(id.to_owned(), (now, count));
    }

    pub fn acknowledged(&mut self, id: &str) {
        self.sent.remove(id);
        self.queued.remove(id);
        self.retries.retain(|(event_id, _)| event_id != id);
    }

    pub fn receipt(&mut self, id: Uuid) -> anyhow::Result<()> {
        if self.receipts.contains(&id) {
            return Ok(());
        }
        // End the connection on genuine overload; the sender's durable outbox
        // can retry on reconnection. Never silently discard a queued receipt.
        anyhow::ensure!(
            self.receipts.len() < MAX_RECEIPTS,
            "确认队列繁忙，请重新连接后重试"
        );
        self.receipts.push_back(id);
        self.receipt_ready.notify_one();
        Ok(())
    }

    pub fn pop_receipt(&mut self) -> Option<Uuid> {
        self.receipts.pop_front()
    }
    pub fn has_receipts(&self) -> bool {
        !self.receipts.is_empty()
    }
    pub fn retry(&mut self, event_id: String, message: Message) -> bool {
        if self.retries.len() >= 4 {
            return false;
        }
        self.queued.insert(event_id.clone());
        self.retries.push_back((event_id, message));
        self.receipt_ready.notify_one();
        true
    }
    pub fn pop_retry(&mut self) -> Option<(String, Message)> {
        self.retries.pop_front()
    }
    pub fn has_retries(&self) -> bool {
        !self.retries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipt_buffer_is_bounded_deduplicated_and_keeps_every_queued_id() {
        let mut state = SessionState::default();
        let ids: Vec<_> = (0..MAX_RECEIPTS).map(|_| Uuid::new_v4()).collect();
        for id in &ids {
            state.receipt(*id).unwrap();
        }
        for id in &ids {
            state.receipt(*id).unwrap();
        }
        assert!(state.receipt(Uuid::new_v4()).is_err());
        for id in ids {
            assert_eq!(state.pop_receipt(), Some(id));
        }
        assert!(state.pop_receipt().is_none());
    }
    #[test]
    fn retry_bookkeeping_is_bounded_and_never_permanently_exhausted() {
        let mut state = SessionState::default();
        let now = Instant::now();
        for i in 0..MAX_TRACKED * 2 {
            state.record_sent(&format!("{i}"), now);
        }
        assert!(state.sent.len() <= MAX_TRACKED);
        for _ in 0..100 {
            state.record_sent("retry", now);
        }
        assert!(!state.due("retry", now));
        assert!(state.due("retry", now + Duration::from_secs(60)));
    }
}
