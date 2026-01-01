use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
pub struct KudosCache {
    entries: HashMap<String, HashSet<Vec<u8>>>,
    pending: HashSet<PendingKudos>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct PendingKudos {
    path: String,
    interaction_id: Vec<u8>,
}

impl KudosCache {
    pub fn insert(&mut self, path: &str, interaction_id: &[u8]) -> bool {
        let entry = self.entries.entry(path.to_string()).or_default();
        if entry.insert(interaction_id.to_vec()) {
            self.pending.insert(PendingKudos {
                path: path.to_string(),
                interaction_id: interaction_id.to_vec(),
            });
            return true;
        }
        false
    }

    pub fn count(&self, path: &str) -> i64 {
        self.entries
            .get(path)
            .map(|set| i64::try_from(set.len()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }

    pub fn total_count(&self) -> i64 {
        let total: usize = self.entries.values().map(|set| set.len()).sum();
        i64::try_from(total).unwrap_or(i64::MAX)
    }

    pub fn path_count(&self) -> i64 {
        i64::try_from(self.entries.len()).unwrap_or(i64::MAX)
    }

    pub fn pending_count(&self) -> i64 {
        i64::try_from(self.pending.len()).unwrap_or(i64::MAX)
    }

    pub fn has(&self, path: &str, interaction_id: &[u8]) -> bool {
        self.entries
            .get(path)
            .map_or(false, |set| set.contains(interaction_id))
    }

    pub fn load_existing<I>(&mut self, items: I) -> usize
    where
        I: IntoIterator<Item = (String, Vec<u8>)>,
    {
        let mut inserted = 0;
        for (path, interaction_id) in items {
            let entry = self.entries.entry(path).or_default();
            if entry.insert(interaction_id) {
                inserted += 1;
            }
        }
        inserted
    }

    pub fn take_pending(&mut self) -> Vec<(String, Vec<u8>)> {
        self.pending
            .drain()
            .map(|pending| (pending.path, pending.interaction_id))
            .collect()
    }

    pub fn restore_pending<I>(&mut self, items: I)
    where
        I: IntoIterator<Item = (String, Vec<u8>)>,
    {
        for (path, interaction_id) in items {
            if self
                .entries
                .get(&path)
                .map_or(false, |set| set.contains(&interaction_id))
            {
                self.pending.insert(PendingKudos { path, interaction_id });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::KudosCache;

    #[test]
    fn insert_tracks_count_and_pending() {
        let mut cache = KudosCache::default();
        assert!(cache.insert("/posts/a", &[1, 2, 3]));
        assert_eq!(cache.count("/posts/a"), 1);
        assert!(cache.has("/posts/a", &[1, 2, 3]));
        let pending = cache.take_pending();
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn load_existing_does_not_mark_pending() {
        let mut cache = KudosCache::default();
        let inserted = cache.load_existing(vec![("/posts/a".to_string(), vec![9, 9])]);
        assert_eq!(inserted, 1);
        assert_eq!(cache.take_pending().len(), 0);
        assert!(cache.has("/posts/a", &[9, 9]));
    }

    #[test]
    fn counts_track_cache_state() {
        let mut cache = KudosCache::default();
        assert_eq!(cache.total_count(), 0);
        assert_eq!(cache.path_count(), 0);
        assert_eq!(cache.pending_count(), 0);
        assert!(cache.insert("/posts/a", &[1]));
        assert!(cache.insert("/posts/b", &[2]));
        assert_eq!(cache.total_count(), 2);
        assert_eq!(cache.path_count(), 2);
        assert_eq!(cache.pending_count(), 2);
    }

    #[test]
    fn restore_pending_skips_missing_entries() {
        let mut cache = KudosCache::default();
        cache.restore_pending(vec![("/posts/a".to_string(), vec![1])]);
        assert!(cache.take_pending().is_empty());
    }
}
