use std::collections::HashMap;
use crate::types::PeerScore;

const BLACKLIST_THRESHOLD: i32 = -10;
const SCORE_DECAY_INTERVAL_SECS: i64 = 3600;

pub struct PeerReputationTable {
    scores: HashMap<String, PeerScore>,
}

impl PeerReputationTable {
    pub fn new() -> Self {
        Self { scores: HashMap::new() }
    }

    pub fn get_score(&self, peer_id: &str) -> Option<i32> {
        self.scores.get(peer_id).map(|s| s.score)
    }

    pub fn record_success(&mut self, peer_id: &str) {
        let entry = self.scores.entry(peer_id.to_string()).or_insert_with(|| PeerScore {
            peer_id: peer_id.to_string(),
            score: 0,
            last_seen: chrono::Utc::now(),
            successful_streams: 0,
            failed_verifications: 0,
            dropped_connections: 0,
            false_claims: 0,
        });
        entry.score += 1;
        entry.successful_streams += 1;
        entry.last_seen = chrono::Utc::now();
        Self::apply_decay(entry);
    }

    pub fn record_corruption(&mut self, peer_id: &str) {
        let entry = self.scores.entry(peer_id.to_string()).or_insert_with(|| PeerScore {
            peer_id: peer_id.to_string(),
            score: 0,
            last_seen: chrono::Utc::now(),
            successful_streams: 0,
            failed_verifications: 0,
            dropped_connections: 0,
            false_claims: 0,
        });
        entry.score -= 3;
        entry.failed_verifications += 1;
        entry.last_seen = chrono::Utc::now();
    }

    pub fn record_dropped(&mut self, peer_id: &str) {
        let entry = self.scores.entry(peer_id.to_string()).or_insert_with(|| PeerScore {
            peer_id: peer_id.to_string(),
            score: 0,
            last_seen: chrono::Utc::now(),
            successful_streams: 0,
            failed_verifications: 0,
            dropped_connections: 0,
            false_claims: 0,
        });
        entry.score -= 1;
        entry.dropped_connections += 1;
        entry.last_seen = chrono::Utc::now();
    }

    pub fn record_false_claim(&mut self, peer_id: &str) {
        let entry = self.scores.entry(peer_id.to_string()).or_insert_with(|| PeerScore {
            peer_id: peer_id.to_string(),
            score: 0,
            last_seen: chrono::Utc::now(),
            successful_streams: 0,
            failed_verifications: 0,
            dropped_connections: 0,
            false_claims: 0,
        });
        entry.score -= 5;
        entry.false_claims += 1;
        entry.last_seen = chrono::Utc::now();
    }

    pub fn is_blacklisted(&self, peer_id: &str) -> bool {
        self.scores.get(peer_id).is_some_and(|s| s.score <= BLACKLIST_THRESHOLD)
    }

    fn apply_decay(entry: &mut PeerScore) {
        let now = chrono::Utc::now();
        let elapsed = now.signed_duration_since(entry.last_seen).num_seconds();
        if elapsed > SCORE_DECAY_INTERVAL_SECS {
            let decays = elapsed / SCORE_DECAY_INTERVAL_SECS;
            if entry.score > 0 {
                entry.score = (entry.score - decays as i32).max(0);
            }
        }
    }

    pub fn get_peer_score(&self, peer_id: &str) -> Option<&PeerScore> {
        self.scores.get(peer_id)
    }

    pub fn peer_count(&self) -> usize {
        self.scores.len()
    }
}

impl Default for PeerReputationTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_peer_starts_at_zero() {
        let table = PeerReputationTable::new();
        assert_eq!(table.get_score("12D3KooWAbc"), None);
    }

    #[test]
    fn test_successful_stream_adds_score() {
        let mut table = PeerReputationTable::new();
        let peer_id = "12D3KooWAbc";
        table.record_success(peer_id);
        assert_eq!(table.get_score(peer_id), Some(1));
        table.record_success(peer_id);
        assert_eq!(table.get_score(peer_id), Some(2));
    }

    #[test]
    fn test_corruption_reduces_score() {
        let mut table = PeerReputationTable::new();
        let peer_id = "12D3KooWAbc";
        table.record_corruption(peer_id);
        assert_eq!(table.get_score(peer_id), Some(-3));
    }

    #[test]
    fn test_blacklist_threshold() {
        let mut table = PeerReputationTable::new();
        let peer_id = "12D3KooWAbc";
        for _ in 0..4 {
            table.record_corruption(peer_id);
        }
        assert_eq!(table.get_score(peer_id), Some(-12));
        assert!(table.is_blacklisted(peer_id));
    }

    #[test]
    fn test_not_blacklisted_above_threshold() {
        let mut table = PeerReputationTable::new();
        let peer_id = "12D3KooWAbc";
        table.record_success(peer_id);
        table.record_corruption(peer_id); // score: 1 - 3 = -2
        assert!(!table.is_blacklisted(peer_id));
    }

    #[test]
    fn test_false_claim_reduces_score() {
        let mut table = PeerReputationTable::new();
        let peer_id = "12D3KooWAbc";
        table.record_false_claim(peer_id);
        assert_eq!(table.get_score(peer_id), Some(-5));
    }

    #[test]
    fn test_dropped_reduces_score() {
        let mut table = PeerReputationTable::new();
        let peer_id = "12D3KooWAbc";
        table.record_dropped(peer_id);
        assert_eq!(table.get_score(peer_id), Some(-1));
    }

    #[test]
    fn test_peer_count() {
        let mut table = PeerReputationTable::new();
        assert_eq!(table.peer_count(), 0);
        table.record_success("peer_a");
        table.record_success("peer_b");
        assert_eq!(table.peer_count(), 2);
    }
}