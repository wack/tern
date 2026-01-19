//! Core types for schema comparison.
//!
//! This module provides generic types for representing differences between
//! two schema snapshots.

use serde::{Deserialize, Serialize};

/// A change in a field's value from source to target.
///
/// Used to represent modifications to individual fields within an object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldChange<T> {
    /// The value in the source schema.
    pub source: T,
    /// The value in the target schema.
    pub target: T,
}

impl<T> FieldChange<T> {
    /// Creates a new field change.
    pub fn new(source: T, target: T) -> Self {
        Self { source, target }
    }
}

impl<T: PartialEq> FieldChange<T> {
    /// Returns `Some(FieldChange)` if the values differ, `None` if they're equal.
    pub fn from_diff(source: T, target: T) -> Option<Self> {
        if source != target {
            Some(Self { source, target })
        } else {
            None
        }
    }
}

/// A potential rename detected by similarity matching.
///
/// When an item is removed from source and a similar item is added in target,
/// this might represent a rename rather than a delete + add.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PotentialRename<K, T> {
    /// The key (name) of the item in the source schema.
    pub source_key: K,
    /// The full item from the target schema.
    pub target: T,
    /// Similarity score from 0.0 (completely different) to 1.0 (identical except name).
    pub similarity: f64,
}

impl<K, T> PotentialRename<K, T> {
    /// Creates a new potential rename.
    pub fn new(source_key: K, target: T, similarity: f64) -> Self {
        Self {
            source_key,
            target,
            similarity,
        }
    }
}

/// A collection of differences for a set of keyed items.
///
/// Generic over:
/// - `K`: The key type (e.g., `TableName`, `ColumnName`)
/// - `T`: The full item type (e.g., `Table`, `Column`)
/// - `M`: The modification details type (e.g., `ModifiedTable`, `ModifiedColumn`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diff<K, T, M> {
    /// Items that exist only in the target schema (newly added).
    pub added: Vec<T>,
    /// Keys of items that exist only in the source schema (removed).
    pub removed: Vec<K>,
    /// Items that exist in both schemas but have differences.
    pub modified: Vec<M>,
    /// Potential renames detected by similarity matching.
    /// Items here are excluded from `added` and `removed`.
    pub potential_renames: Vec<PotentialRename<K, T>>,
}

impl<K, T, M> Default for Diff<K, T, M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, T, M> Diff<K, T, M> {
    /// Creates an empty diff.
    pub fn new() -> Self {
        Self {
            added: Vec::new(),
            removed: Vec::new(),
            modified: Vec::new(),
            potential_renames: Vec::new(),
        }
    }

    /// Returns true if there are no differences.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.modified.is_empty()
            && self.potential_renames.is_empty()
    }

    /// Returns the total number of changes.
    pub fn change_count(&self) -> usize {
        self.added.len()
            + self.removed.len()
            + self.modified.len()
            + self.potential_renames.len()
    }
}

/// Configuration for the diff algorithm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffConfig {
    /// Minimum similarity score (0.0-1.0) for rename detection.
    /// Items with similarity below this threshold will be treated as add/remove.
    /// Set to 1.0 to disable rename detection entirely.
    pub rename_threshold: f64,
}

impl Default for DiffConfig {
    fn default() -> Self {
        Self {
            // Default: require 50% structural similarity for rename detection
            rename_threshold: 0.5,
        }
    }
}

impl DiffConfig {
    /// Creates a config that disables rename detection.
    pub fn no_rename_detection() -> Self {
        Self {
            rename_threshold: 1.0,
        }
    }

    /// Creates a config with a custom rename threshold.
    pub fn with_rename_threshold(threshold: f64) -> Self {
        Self {
            rename_threshold: threshold.clamp(0.0, 1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_change_from_diff_some() {
        let change = FieldChange::from_diff(1, 2);
        assert!(change.is_some());
        let change = change.unwrap();
        assert_eq!(change.source, 1);
        assert_eq!(change.target, 2);
    }

    #[test]
    fn field_change_from_diff_none() {
        let change = FieldChange::from_diff(42, 42);
        assert!(change.is_none());
    }

    #[test]
    fn diff_is_empty() {
        let diff: Diff<String, String, String> = Diff::new();
        assert!(diff.is_empty());
        assert_eq!(diff.change_count(), 0);
    }

    #[test]
    fn diff_change_count() {
        let mut diff: Diff<String, String, String> = Diff::new();
        diff.added.push("a".to_string());
        diff.removed.push("b".to_string());
        diff.modified.push("c".to_string());
        assert!(!diff.is_empty());
        assert_eq!(diff.change_count(), 3);
    }

    #[test]
    fn config_defaults() {
        let config = DiffConfig::default();
        assert_eq!(config.rename_threshold, 0.5);
    }

    #[test]
    fn config_no_rename() {
        let config = DiffConfig::no_rename_detection();
        assert_eq!(config.rename_threshold, 1.0);
    }

    #[test]
    fn config_threshold_clamped() {
        let config = DiffConfig::with_rename_threshold(1.5);
        assert_eq!(config.rename_threshold, 1.0);

        let config = DiffConfig::with_rename_threshold(-0.5);
        assert_eq!(config.rename_threshold, 0.0);
    }
}
