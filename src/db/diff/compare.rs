//! Schema comparison implementation.
//!
//! This module provides functions for comparing schema objects and producing diffs.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use crate::db::model::{
    Column, Constraint, ConstraintKind, EnumType, Index, Namespace, Sequence, Table, View,
};
use crate::db::schema::ColumnName;

use super::schema_diff::{
    ColumnDiff, ConstraintDiff, EnumDiff, IndexDiff, ModifiedColumn, ModifiedConstraint,
    ModifiedEnum, ModifiedIndex, ModifiedSequence, ModifiedTable, ModifiedView, NamespaceDiff,
    SequenceDiff, TableDiff, ViewDiff,
};
use super::types::{Diff, DiffConfig, FieldChange};

// =============================================================================
// Public API
// =============================================================================

/// Compares two namespaces and returns a diff.
///
/// Uses the default configuration (rename threshold = 0.5).
pub fn diff_namespaces(source: &Namespace, target: &Namespace) -> NamespaceDiff {
    diff_namespaces_with_config(source, target, &DiffConfig::default())
}

/// Compares two namespaces with a custom configuration.
pub fn diff_namespaces_with_config(
    source: &Namespace,
    target: &Namespace,
    config: &DiffConfig,
) -> NamespaceDiff {
    NamespaceDiff {
        name: target.name.clone(),
        tables: diff_tables(&source.tables, &target.tables, config),
        views: diff_views(&source.views, &target.views, config),
        sequences: diff_sequences(&source.sequences, &target.sequences, config),
        enums: diff_enums(&source.enums, &target.enums, config),
        comment: FieldChange::from_diff(source.comment.clone(), target.comment.clone()),
    }
}

// =============================================================================
// Table Diffing
// =============================================================================

fn diff_tables(source: &[Table], target: &[Table], config: &DiffConfig) -> TableDiff {
    diff_by_key(
        source,
        target,
        |t| t.name.clone(),
        |s, t| diff_table(s, t, config),
        table_similarity,
        config,
    )
}

fn diff_table(source: &Table, target: &Table, config: &DiffConfig) -> Option<ModifiedTable> {
    let kind = FieldChange::from_diff(source.kind, target.kind);
    let columns = diff_columns(&source.columns, &target.columns, config);
    let constraints = diff_constraints(&source.constraints, &target.constraints, config);
    let indexes = diff_indexes(&source.indexes, &target.indexes, config);
    let comment = FieldChange::from_diff(source.comment.clone(), target.comment.clone());

    let modified = ModifiedTable {
        name: target.name.clone(),
        kind,
        columns,
        constraints,
        indexes,
        comment,
    };

    if modified.is_empty() {
        None
    } else {
        Some(modified)
    }
}

/// Calculates similarity between two tables (0.0 to 1.0).
///
/// Factors considered:
/// - Column name overlap (weighted most heavily)
/// - Constraint similarity
/// - Column type matches for matching names
fn table_similarity(source: &Table, target: &Table) -> f64 {
    if source.columns.is_empty() && target.columns.is_empty() {
        // Both empty - only similar if same kind
        return if source.kind == target.kind { 0.5 } else { 0.0 };
    }

    if source.columns.is_empty() || target.columns.is_empty() {
        // One empty, one not - very dissimilar
        return 0.0;
    }

    let source_cols: HashSet<&str> = source.columns.iter().map(|c| c.name.as_ref()).collect();
    let target_cols: HashSet<&str> = target.columns.iter().map(|c| c.name.as_ref()).collect();

    // Jaccard similarity for column names
    let intersection = source_cols.intersection(&target_cols).count();
    let union = source_cols.union(&target_cols).count();
    let name_similarity = intersection as f64 / union as f64;

    // Type match ratio for columns with matching names
    let type_matches = source
        .columns
        .iter()
        .filter(|sc| {
            target
                .columns
                .iter()
                .any(|tc| tc.name == sc.name && tc.type_info == sc.type_info)
        })
        .count();
    let type_similarity = if intersection > 0 {
        type_matches as f64 / intersection as f64
    } else {
        0.0
    };

    // Weighted combination: 70% column names, 30% type matches
    name_similarity * 0.7 + type_similarity * 0.3
}

// =============================================================================
// Column Diffing
// =============================================================================

fn diff_columns(source: &[Column], target: &[Column], config: &DiffConfig) -> ColumnDiff {
    diff_by_key(
        source,
        target,
        |c| c.name.clone(),
        diff_column,
        column_similarity,
        config,
    )
}

fn diff_column(source: &Column, target: &Column) -> Option<ModifiedColumn> {
    let position = FieldChange::from_diff(source.position, target.position);
    let type_info = FieldChange::from_diff(source.type_info.clone(), target.type_info.clone());
    let is_nullable = FieldChange::from_diff(source.is_nullable, target.is_nullable);
    let default = FieldChange::from_diff(source.default.clone(), target.default.clone());
    let generated = FieldChange::from_diff(source.generated.clone(), target.generated.clone());
    let identity = FieldChange::from_diff(source.identity, target.identity);
    let collation = FieldChange::from_diff(source.collation.clone(), target.collation.clone());
    let comment = FieldChange::from_diff(source.comment.clone(), target.comment.clone());

    let modified = ModifiedColumn {
        name: target.name.clone(),
        position,
        type_info,
        is_nullable,
        default,
        generated,
        identity,
        collation,
        comment,
    };

    if modified.is_empty() {
        None
    } else {
        Some(modified)
    }
}

/// Calculates similarity between two columns (0.0 to 1.0).
///
/// Factors considered:
/// - Same type (most important - required for high similarity)
/// - Same nullability
/// - Same default
/// - Position proximity
///
/// Type mismatch is a strong signal: if types don't match, the maximum
/// possible score is 0.35, which is below the default rename threshold.
fn column_similarity(source: &Column, target: &Column) -> f64 {
    // Type match is critical for column rename detection.
    // If types don't match, cap the maximum similarity at 0.35.
    let type_matches = source.type_info == target.type_info;

    if !type_matches {
        // Different types: very unlikely to be a rename.
        // Give partial credit for position proximity only.
        let pos_diff = (source.position - target.position).abs() as f64;
        let pos_score = 1.0 / (1.0 + pos_diff * 0.5);
        return 0.35 * pos_score;
    }

    // Types match: calculate full similarity
    let mut score = 0.6; // Base score for type match

    // Nullability match: 15% weight
    if source.is_nullable == target.is_nullable {
        score += 0.15;
    }

    // Default match: 10% weight
    if source.default == target.default {
        score += 0.1;
    }

    // Position proximity: 15% weight
    let pos_diff = (source.position - target.position).abs() as f64;
    let pos_score = 1.0 / (1.0 + pos_diff * 0.2);
    score += 0.15 * pos_score;

    score
}

// =============================================================================
// Constraint Diffing
// =============================================================================

fn diff_constraints(
    source: &[Constraint],
    target: &[Constraint],
    config: &DiffConfig,
) -> ConstraintDiff {
    diff_by_key(
        source,
        target,
        |c| c.name.clone(),
        diff_constraint,
        constraint_similarity,
        config,
    )
}

fn diff_constraint(source: &Constraint, target: &Constraint) -> Option<ModifiedConstraint> {
    // Compare the constraint kinds structurally
    let kind_changed = source.kind != target.kind;
    let comment_changed = source.comment != target.comment;

    if kind_changed || comment_changed {
        Some(ModifiedConstraint {
            name: target.name.clone(),
            source: source.clone(),
            target: target.clone(),
            comment: FieldChange::from_diff(source.comment.clone(), target.comment.clone()),
        })
    } else {
        None
    }
}

/// Calculates similarity between two constraints (0.0 to 1.0).
fn constraint_similarity(source: &Constraint, target: &Constraint) -> f64 {
    // Must be same constraint type
    let same_type = matches!(
        (&source.kind, &target.kind),
        (ConstraintKind::PrimaryKey(_), ConstraintKind::PrimaryKey(_))
            | (ConstraintKind::ForeignKey(_), ConstraintKind::ForeignKey(_))
            | (ConstraintKind::Unique(_), ConstraintKind::Unique(_))
            | (ConstraintKind::Check(_), ConstraintKind::Check(_))
            | (ConstraintKind::Exclusion(_), ConstraintKind::Exclusion(_))
    );

    if !same_type {
        return 0.0;
    }

    // For same type, compare columns/expression
    match (&source.kind, &target.kind) {
        (ConstraintKind::PrimaryKey(s), ConstraintKind::PrimaryKey(t)) => {
            column_list_similarity(&s.columns, &t.columns)
        }
        (ConstraintKind::Unique(s), ConstraintKind::Unique(t)) => {
            column_list_similarity(&s.columns, &t.columns)
        }
        (ConstraintKind::ForeignKey(s), ConstraintKind::ForeignKey(t)) => {
            let col_sim = column_list_similarity(&s.columns, &t.columns);
            let ref_sim = if s.referenced_table == t.referenced_table { 0.5 } else { 0.0 };
            col_sim * 0.5 + ref_sim
        }
        (ConstraintKind::Check(s), ConstraintKind::Check(t)) => {
            if s.expression == t.expression { 1.0 } else { 0.3 }
        }
        (ConstraintKind::Exclusion(s), ConstraintKind::Exclusion(t)) => {
            if s.elements == t.elements { 1.0 } else { 0.3 }
        }
        _ => 0.0,
    }
}

fn column_list_similarity(source: &[ColumnName], target: &[ColumnName]) -> f64 {
    if source.is_empty() && target.is_empty() {
        return 1.0;
    }
    if source.is_empty() || target.is_empty() {
        return 0.0;
    }

    let source_set: HashSet<&ColumnName> = source.iter().collect();
    let target_set: HashSet<&ColumnName> = target.iter().collect();

    let intersection = source_set.intersection(&target_set).count();
    let union = source_set.union(&target_set).count();

    intersection as f64 / union as f64
}

// =============================================================================
// Index Diffing
// =============================================================================

fn diff_indexes(source: &[Index], target: &[Index], config: &DiffConfig) -> IndexDiff {
    diff_by_key(
        source,
        target,
        |i| i.name.clone(),
        diff_index,
        index_similarity,
        config,
    )
}

fn diff_index(source: &Index, target: &Index) -> Option<ModifiedIndex> {
    // Compare indexes structurally (excluding OID and name)
    let structure_changed = source.method != target.method
        || source.is_unique != target.is_unique
        || source.is_constraint_index != target.is_constraint_index
        || source.columns != target.columns
        || source.predicate != target.predicate;

    let comment_changed = source.comment != target.comment;

    if structure_changed || comment_changed {
        Some(ModifiedIndex {
            name: target.name.clone(),
            source: source.clone(),
            target: target.clone(),
            comment: FieldChange::from_diff(source.comment.clone(), target.comment.clone()),
        })
    } else {
        None
    }
}

/// Calculates similarity between two indexes (0.0 to 1.0).
fn index_similarity(source: &Index, target: &Index) -> f64 {
    let mut score = 0.0;

    // Same method: 20%
    if source.method == target.method {
        score += 0.2;
    }

    // Same uniqueness: 15%
    if source.is_unique == target.is_unique {
        score += 0.15;
    }

    // Column overlap: 50%
    let source_cols: Vec<_> = source.columns.iter().filter_map(|c| c.column.as_ref()).collect();
    let target_cols: Vec<_> = target.columns.iter().filter_map(|c| c.column.as_ref()).collect();

    if !source_cols.is_empty() || !target_cols.is_empty() {
        let source_set: HashSet<_> = source_cols.iter().collect();
        let target_set: HashSet<_> = target_cols.iter().collect();
        let intersection = source_set.intersection(&target_set).count();
        let union = source_set.union(&target_set).count();
        if union > 0 {
            score += 0.5 * (intersection as f64 / union as f64);
        }
    } else {
        score += 0.5; // Both have no simple columns (expression indexes)
    }

    // Same predicate: 15%
    if source.predicate == target.predicate {
        score += 0.15;
    }

    score
}

// =============================================================================
// View Diffing
// =============================================================================

fn diff_views(source: &[View], target: &[View], config: &DiffConfig) -> ViewDiff {
    diff_by_key(
        source,
        target,
        |v| v.name.clone(),
        diff_view,
        view_similarity,
        config,
    )
}

fn diff_view(source: &View, target: &View) -> Option<ModifiedView> {
    let definition = FieldChange::from_diff(source.definition.clone(), target.definition.clone());
    let is_materialized = FieldChange::from_diff(source.is_materialized, target.is_materialized);
    let comment = FieldChange::from_diff(source.comment.clone(), target.comment.clone());

    let modified = ModifiedView {
        name: target.name.clone(),
        definition,
        is_materialized,
        comment,
    };

    if modified.is_empty() {
        None
    } else {
        Some(modified)
    }
}

fn view_similarity(source: &View, target: &View) -> f64 {
    let mut score = 0.0;

    // Same materialization: 30%
    if source.is_materialized == target.is_materialized {
        score += 0.3;
    }

    // Definition similarity: 70% (simple check - could be smarter)
    if source.definition == target.definition {
        score += 0.7;
    } else {
        // Partial credit for similar length definitions
        let len_ratio = source.definition.as_ref().len().min(target.definition.as_ref().len())
            as f64
            / source.definition.as_ref().len().max(target.definition.as_ref().len()) as f64;
        score += 0.3 * len_ratio;
    }

    score
}

// =============================================================================
// Sequence Diffing
// =============================================================================

fn diff_sequences(source: &[Sequence], target: &[Sequence], config: &DiffConfig) -> SequenceDiff {
    diff_by_key(
        source,
        target,
        |s| s.name.clone(),
        diff_sequence,
        sequence_similarity,
        config,
    )
}

fn diff_sequence(source: &Sequence, target: &Sequence) -> Option<ModifiedSequence> {
    let data_type = FieldChange::from_diff(source.data_type.clone(), target.data_type.clone());
    let start_value = FieldChange::from_diff(source.start_value, target.start_value);
    let increment = FieldChange::from_diff(source.increment, target.increment);
    let min_value = FieldChange::from_diff(source.min_value, target.min_value);
    let max_value = FieldChange::from_diff(source.max_value, target.max_value);
    let cache_size = FieldChange::from_diff(source.cache_size, target.cache_size);
    let is_cyclic = FieldChange::from_diff(source.is_cyclic, target.is_cyclic);
    let comment = FieldChange::from_diff(source.comment.clone(), target.comment.clone());

    let modified = ModifiedSequence {
        name: target.name.clone(),
        data_type,
        start_value,
        increment,
        min_value,
        max_value,
        cache_size,
        is_cyclic,
        comment,
    };

    if modified.is_empty() {
        None
    } else {
        Some(modified)
    }
}

fn sequence_similarity(source: &Sequence, target: &Sequence) -> f64 {
    let mut score = 0.0;

    // Same data type: 50%
    if source.data_type == target.data_type {
        score += 0.5;
    }

    // Same increment: 25%
    if source.increment == target.increment {
        score += 0.25;
    }

    // Same bounds: 25%
    if source.min_value == target.min_value && source.max_value == target.max_value {
        score += 0.25;
    }

    score
}

// =============================================================================
// Enum Diffing
// =============================================================================

fn diff_enums(source: &[EnumType], target: &[EnumType], config: &DiffConfig) -> EnumDiff {
    diff_by_key(
        source,
        target,
        |e| e.name.clone(),
        diff_enum,
        enum_similarity,
        config,
    )
}

fn diff_enum(source: &EnumType, target: &EnumType) -> Option<ModifiedEnum> {
    let source_values: HashSet<&String> = source.values.iter().collect();
    let target_values: HashSet<&String> = target.values.iter().collect();

    let values_added: Vec<String> = target
        .values
        .iter()
        .filter(|v| !source_values.contains(v))
        .cloned()
        .collect();

    let values_removed: Vec<String> = source
        .values
        .iter()
        .filter(|v| !target_values.contains(v))
        .cloned()
        .collect();

    // Check if common values are in the same order
    let common_source: Vec<&String> = source.values.iter().filter(|v| target_values.contains(v)).collect();
    let common_target: Vec<&String> = target.values.iter().filter(|v| source_values.contains(v)).collect();
    let values_reordered = common_source != common_target;

    let comment = FieldChange::from_diff(source.comment.clone(), target.comment.clone());

    let modified = ModifiedEnum {
        name: target.name.clone(),
        values_added,
        values_removed,
        values_reordered,
        comment,
    };

    if modified.is_empty() {
        None
    } else {
        Some(modified)
    }
}

fn enum_similarity(source: &EnumType, target: &EnumType) -> f64 {
    if source.values.is_empty() && target.values.is_empty() {
        return 1.0;
    }
    if source.values.is_empty() || target.values.is_empty() {
        return 0.0;
    }

    let source_set: HashSet<&String> = source.values.iter().collect();
    let target_set: HashSet<&String> = target.values.iter().collect();

    let intersection = source_set.intersection(&target_set).count();
    let union = source_set.union(&target_set).count();

    intersection as f64 / union as f64
}

// =============================================================================
// Generic Diffing Helper
// =============================================================================

/// Generic function for diffing two slices of items by a key.
///
/// - `source`: Items from the source schema
/// - `target`: Items from the target schema
/// - `key_fn`: Extracts the key from an item
/// - `diff_fn`: Compares two items with the same key, returns `None` if identical
/// - `similarity_fn`: Calculates similarity between two items (for rename detection)
/// - `config`: Diff configuration
fn diff_by_key<T, K, M, KeyFn, DiffFn, SimFn>(
    source: &[T],
    target: &[T],
    key_fn: KeyFn,
    diff_fn: DiffFn,
    similarity_fn: SimFn,
    config: &DiffConfig,
) -> Diff<K, T, M>
where
    T: Clone,
    K: Clone + Eq + Hash,
    KeyFn: Fn(&T) -> K,
    DiffFn: Fn(&T, &T) -> Option<M>,
    SimFn: Fn(&T, &T) -> f64,
{
    let source_map: HashMap<K, &T> = source.iter().map(|item| (key_fn(item), item)).collect();
    let target_map: HashMap<K, &T> = target.iter().map(|item| (key_fn(item), item)).collect();

    let source_keys: HashSet<&K> = source_map.keys().collect();
    let target_keys: HashSet<&K> = target_map.keys().collect();

    // Items present in both
    let mut modified = Vec::new();
    for key in source_keys.intersection(&target_keys) {
        let source_item = source_map[*key];
        let target_item = target_map[*key];
        if let Some(diff) = diff_fn(source_item, target_item) {
            modified.push(diff);
        }
    }

    // Items only in source (removed) or only in target (added)
    let mut only_in_source: Vec<K> = source_keys
        .difference(&target_keys)
        .map(|k| (*k).clone())
        .collect();
    let mut only_in_target: Vec<&T> = target_keys
        .difference(&source_keys)
        .map(|k| *target_map.get(*k).unwrap())
        .collect();

    // Rename detection
    let mut potential_renames = Vec::new();

    if config.rename_threshold < 1.0 && !only_in_source.is_empty() && !only_in_target.is_empty() {
        // Find best matches for potential renames
        let mut used_sources: HashSet<K> = HashSet::new();
        let mut used_targets: HashSet<K> = HashSet::new();

        // Calculate all similarities and sort by score
        let mut candidates: Vec<(K, K, &T, f64)> = Vec::new();
        for source_key in &only_in_source {
            let source_item = source_map[source_key];
            for target_item in &only_in_target {
                let target_key = key_fn(target_item);
                let sim = similarity_fn(source_item, target_item);
                if sim >= config.rename_threshold {
                    candidates.push((source_key.clone(), target_key, target_item, sim));
                }
            }
        }

        // Sort by similarity descending
        candidates.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));

        // Greedily assign best matches
        for (source_key, target_key, target_item, sim) in candidates {
            if !used_sources.contains(&source_key) && !used_targets.contains(&target_key) {
                potential_renames.push(super::types::PotentialRename::new(
                    source_key.clone(),
                    (*target_item).clone(),
                    sim,
                ));
                used_sources.insert(source_key);
                used_targets.insert(target_key);
            }
        }

        // Remove matched items from added/removed lists
        only_in_source.retain(|k| !used_sources.contains(k));
        only_in_target.retain(|item| !used_targets.contains(&key_fn(item)));
    }

    Diff {
        added: only_in_target.into_iter().cloned().collect(),
        removed: only_in_source,
        modified,
        potential_renames,
    }
}
