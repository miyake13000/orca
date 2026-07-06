//! [`CommitBuilder`]: computes the immutable commit hash.

use chrono::{DateTime, SecondsFormat, Utc};
use orca_hash::{Hashable, Hasher, Sha256Hasher};

use crate::VcsError;
use crate::data::Commit;

/// Builder that assembles a [`Commit`] and computes its immutable hash.
///
/// Holds `&'a dyn Hashable` references until [`build`](Self::build) so that
/// all fields are streamed into a single hasher in a fixed order
/// (`parents → message → timestamp → data`) regardless of the order the
/// builder methods were called in — this keeps the hash deterministic.
///
/// The hash formula is
/// `SHA256(parent_hashes ‖ framed(message) ‖ framed(timestamp) ‖ data)`
/// where `data` is the tree walk of an `Upper` (normal commit) or `Layer`.
/// The hash is computed exactly once here and never recomputed afterwards
/// (rebase only rewrites parent pointers).
pub struct CommitBuilder<'a> {
    parents: Vec<orca_hash::Hash>,
    message: String,
    timestamp: Option<DateTime<Utc>>,
    data: Option<&'a dyn Hashable>,
}

impl<'a> CommitBuilder<'a> {
    /// Start building a commit with no parents, empty message, and no data.
    pub fn new() -> Self {
        Self {
            parents: Vec::new(),
            message: String::new(),
            timestamp: None,
            data: None,
        }
    }

    /// Add a parent commit hash (call once per parent, in order).
    pub fn parent(mut self, hash: orca_hash::Hash) -> Self {
        self.parents.push(hash);
        self
    }

    /// Set the commit message.
    pub fn message(mut self, msg: &str) -> Self {
        self.message = msg.to_string();
        self
    }

    /// Attach the commit payload (normally `&Upper`; `&Layer` for internal
    /// use). Its tree walk is streamed into the hash, and the resulting
    /// commit will reference a layer named after the commit hash.
    pub fn data(mut self, obj: &'a dyn Hashable) -> Self {
        self.data = Some(obj);
        self
    }

    /// Set the timestamp to the current time.
    pub fn timestamp_now(mut self) -> Self {
        self.timestamp = Some(Utc::now());
        self
    }

    /// Set an explicit timestamp (mainly for tests).
    pub fn timestamp(mut self, ts: DateTime<Utc>) -> Self {
        self.timestamp = Some(ts);
        self
    }

    /// Compute the immutable hash and produce the [`Commit`].
    ///
    /// The returned commit's `layer` is `Some(hash)` iff data was attached;
    /// the hash doubles as the layer directory name (`layers/<hash>/`).
    /// Uses the current time if no timestamp was set.
    ///
    /// # Panics
    ///
    /// Propagates panics from the attached [`Hashable`] (e.g. an `Upper`
    /// whose directory walk hits an I/O error).
    pub fn build(self) -> Result<Commit, VcsError> {
        let timestamp = self.timestamp.unwrap_or_else(Utc::now);
        let mut hasher = Sha256Hasher::new();
        for parent in &self.parents {
            Hashable::hash(parent, &mut hasher);
        }
        hasher.update_framed(self.message.as_bytes());
        hasher.update_framed(
            timestamp
                .to_rfc3339_opts(SecondsFormat::Nanos, true)
                .as_bytes(),
        );
        if let Some(data) = self.data {
            data.hash(&mut hasher);
        }
        let hash = hasher.finalize();
        Ok(Commit {
            hash,
            parent: self.parents,
            layer: self.data.is_some().then_some(hash),
            message: self.message,
            timestamp,
        })
    }
}

impl Default for CommitBuilder<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orca_hash::Hasher;

    struct Fixed(&'static [u8]);
    impl Hashable for Fixed {
        fn hash(&self, hasher: &mut dyn Hasher) {
            hasher.update_framed(self.0);
        }
    }

    #[test]
    fn hash_is_deterministic_and_order_independent() {
        let ts = Utc::now();
        let data = Fixed(b"layer");
        let a = CommitBuilder::new()
            .message("msg")
            .parent([1; 32])
            .data(&data)
            .timestamp(ts)
            .build()
            .unwrap();
        let b = CommitBuilder::new()
            .parent([1; 32])
            .timestamp(ts)
            .data(&data)
            .message("msg")
            .build()
            .unwrap();
        assert_eq!(a.hash, b.hash);
    }

    #[test]
    fn different_fields_change_hash() {
        let ts = Utc::now();
        let base = CommitBuilder::new().message("a").timestamp(ts).build().unwrap();
        let other = CommitBuilder::new().message("b").timestamp(ts).build().unwrap();
        assert_ne!(base.hash, other.hash);
    }

    #[test]
    fn layer_follows_data_presence() {
        let no_data = CommitBuilder::new().timestamp_now().build().unwrap();
        assert!(no_data.layer.is_none());
        assert!(no_data.is_root());

        let data = Fixed(b"x");
        let with_data = CommitBuilder::new()
            .parent(no_data.hash)
            .data(&data)
            .timestamp_now()
            .build()
            .unwrap();
        assert_eq!(with_data.layer, Some(with_data.hash));
        assert!(!with_data.is_root());
    }
}
