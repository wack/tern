//! Advanced similarity scoring for rename detection.
//!
//! This module provides configurable similarity scoring algorithms for detecting
//! potential renames of schema objects (tables, columns, etc.).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::db::model::{Column, ConstraintKind, Index, Table};

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for rename detection similarity scoring.
///
/// Controls how different factors contribute to the overall similarity score
/// when determining if two schema objects might be renames of each other.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarityConfig {
    /// Weight for column name overlap (Jaccard similarity).
    /// Range: 0.0-1.0, default: 0.35
    pub column_name_weight: f64,

    /// Weight for type matches among overlapping columns.
    /// Range: 0.0-1.0, default: 0.25
    pub type_match_weight: f64,

    /// Weight for constraint similarity (PK, FK, unique structure).
    /// Range: 0.0-1.0, default: 0.15
    pub constraint_weight: f64,

    /// Weight for index structure similarity.
    /// Range: 0.0-1.0, default: 0.10
    pub index_weight: f64,

    /// Weight for name similarity (edit distance).
    /// Range: 0.0-1.0, default: 0.15
    pub name_similarity_weight: f64,

    /// Bonus multiplier for primary key column matches.
    /// Columns that are part of a PK are weighted more heavily.
    /// Range: 1.0-3.0, default: 1.5
    pub pk_column_bonus: f64,

    /// Bonus multiplier for foreign key column matches.
    /// Columns referenced by FKs indicate structural importance.
    /// Range: 1.0-2.0, default: 1.3
    pub fk_column_bonus: f64,
}

impl Default for SimilarityConfig {
    fn default() -> Self {
        Self {
            column_name_weight: 0.35,
            type_match_weight: 0.25,
            constraint_weight: 0.15,
            index_weight: 0.10,
            name_similarity_weight: 0.15,
            pk_column_bonus: 1.5,
            fk_column_bonus: 1.3,
        }
    }
}

impl SimilarityConfig {
    /// Creates a config that ignores name similarity (pure structural comparison).
    pub fn structural_only() -> Self {
        Self {
            column_name_weight: 0.45,
            type_match_weight: 0.30,
            constraint_weight: 0.15,
            index_weight: 0.10,
            name_similarity_weight: 0.0,
            ..Default::default()
        }
    }

    /// Creates a config that heavily weights name similarity.
    pub fn name_focused() -> Self {
        Self {
            column_name_weight: 0.25,
            type_match_weight: 0.20,
            constraint_weight: 0.10,
            index_weight: 0.05,
            name_similarity_weight: 0.40,
            ..Default::default()
        }
    }

    /// Normalizes weights to sum to 1.0.
    fn normalized_weights(&self) -> (f64, f64, f64, f64, f64) {
        let total = self.column_name_weight
            + self.type_match_weight
            + self.constraint_weight
            + self.index_weight
            + self.name_similarity_weight;

        if total == 0.0 {
            return (0.2, 0.2, 0.2, 0.2, 0.2);
        }

        (
            self.column_name_weight / total,
            self.type_match_weight / total,
            self.constraint_weight / total,
            self.index_weight / total,
            self.name_similarity_weight / total,
        )
    }
}

// =============================================================================
// Name Similarity (Edit Distance)
// =============================================================================

/// Calculates the Levenshtein edit distance between two strings.
///
/// Returns the minimum number of single-character edits (insertions,
/// deletions, or substitutions) required to change one string into the other.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    // Use two rows instead of full matrix for space efficiency
    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row: Vec<usize> = vec![0; b_len + 1];

    for (i, a_char) in a_chars.iter().enumerate() {
        curr_row[0] = i + 1;

        for (j, b_char) in b_chars.iter().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };

            curr_row[j + 1] = (prev_row[j + 1] + 1) // deletion
                .min(curr_row[j] + 1) // insertion
                .min(prev_row[j] + cost); // substitution
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

/// Calculates normalized name similarity between two strings (0.0 to 1.0).
///
/// Uses Levenshtein distance normalized by the length of the longer string.
/// Also considers common prefixes/suffixes and word boundaries.
pub fn name_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }

    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let a_lower = a.to_lowercase();
    let b_lower = b.to_lowercase();

    // Check for common naming patterns
    let pattern_bonus = compute_pattern_bonus(&a_lower, &b_lower);

    // Compute normalized Levenshtein similarity
    let distance = levenshtein_distance(&a_lower, &b_lower);
    let max_len = a_lower.len().max(b_lower.len());
    let edit_similarity = 1.0 - (distance as f64 / max_len as f64);

    // Combine edit distance similarity with pattern bonus
    (edit_similarity * 0.7 + pattern_bonus * 0.3).min(1.0)
}

/// Computes bonus for common naming patterns.
fn compute_pattern_bonus(a: &str, b: &str) -> f64 {
    let mut bonus = 0.0;

    // Common prefix bonus
    let common_prefix_len = a
        .chars()
        .zip(b.chars())
        .take_while(|(ca, cb)| ca == cb)
        .count();
    if common_prefix_len >= 3 {
        bonus += 0.3 * (common_prefix_len as f64 / a.len().min(b.len()) as f64);
    }

    // Common suffix bonus
    let common_suffix_len = a
        .chars()
        .rev()
        .zip(b.chars().rev())
        .take_while(|(ca, cb)| ca == cb)
        .count();
    if common_suffix_len >= 3 {
        bonus += 0.3 * (common_suffix_len as f64 / a.len().min(b.len()) as f64);
    }

    // Check if one is a substring of the other (common for renames like users -> user_accounts)
    if a.contains(b) || b.contains(a) {
        let shorter = a.len().min(b.len());
        let longer = a.len().max(b.len());
        bonus += 0.4 * (shorter as f64 / longer as f64);
    }

    // Split by common delimiters and check word overlap
    let a_words: HashSet<&str> = a.split(['_', '-']).collect();
    let b_words: HashSet<&str> = b.split(['_', '-']).collect();

    if !a_words.is_empty() && !b_words.is_empty() {
        let intersection = a_words.intersection(&b_words).count();
        let union = a_words.union(&b_words).count();
        if union > 0 {
            bonus += 0.3 * (intersection as f64 / union as f64);
        }
    }

    bonus.min(1.0)
}

// =============================================================================
// Table Similarity
// =============================================================================

/// Calculates comprehensive similarity between two tables (0.0 to 1.0).
///
/// Considers multiple factors with configurable weights:
/// - Column name overlap (Jaccard similarity)
/// - Type matches for overlapping columns
/// - Constraint similarity
/// - Index structure similarity
/// - Name similarity
pub fn table_similarity(source: &Table, target: &Table, config: &SimilarityConfig) -> f64 {
    // Handle edge cases
    if source.columns.is_empty() && target.columns.is_empty() {
        // Both empty tables - use name similarity only
        return name_similarity(source.name.as_ref(), target.name.as_ref());
    }

    if source.columns.is_empty() || target.columns.is_empty() {
        // One empty, one not - very low similarity
        return 0.1 * name_similarity(source.name.as_ref(), target.name.as_ref());
    }

    let (col_weight, type_weight, constraint_weight, index_weight, name_weight) =
        config.normalized_weights();

    // 1. Column name overlap (Jaccard similarity with importance weighting)
    let column_sim = weighted_column_similarity(source, target, config);

    // 2. Type match ratio for columns with matching names
    let type_sim = type_match_similarity(source, target);

    // 3. Constraint similarity
    let constraint_sim = constraint_structure_similarity(source, target);

    // 4. Index similarity
    let index_sim = index_structure_similarity(source, target);

    // 5. Name similarity
    let name_sim = name_similarity(source.name.as_ref(), target.name.as_ref());

    // Weighted combination
    col_weight * column_sim
        + type_weight * type_sim
        + constraint_weight * constraint_sim
        + index_weight * index_sim
        + name_weight * name_sim
}

/// Calculates column name overlap with importance weighting.
fn weighted_column_similarity(source: &Table, target: &Table, config: &SimilarityConfig) -> f64 {
    let source_pk_cols = get_primary_key_columns(source);
    let source_fk_cols = get_foreign_key_columns(source);
    let target_pk_cols = get_primary_key_columns(target);
    let target_fk_cols = get_foreign_key_columns(target);

    let mut source_weighted_count = 0.0;
    let mut target_weighted_count = 0.0;
    let mut intersection_weighted = 0.0;

    let source_col_names: HashSet<&str> = source.columns.iter().map(|c| c.name.as_ref()).collect();
    let target_col_names: HashSet<&str> = target.columns.iter().map(|c| c.name.as_ref()).collect();

    // Calculate weighted counts for source
    for col in &source.columns {
        let name = col.name.as_ref();
        let mut weight = 1.0;
        if source_pk_cols.contains(name) {
            weight *= config.pk_column_bonus;
        }
        if source_fk_cols.contains(name) {
            weight *= config.fk_column_bonus;
        }
        source_weighted_count += weight;

        if target_col_names.contains(name) {
            intersection_weighted += weight;
        }
    }

    // Calculate weighted counts for target
    for col in &target.columns {
        let name = col.name.as_ref();
        let mut weight = 1.0;
        if target_pk_cols.contains(name) {
            weight *= config.pk_column_bonus;
        }
        if target_fk_cols.contains(name) {
            weight *= config.fk_column_bonus;
        }
        target_weighted_count += weight;

        // Only add to intersection if not already counted from source side
        if source_col_names.contains(name) {
            // Intersection already counted from source, but add target's weight
            // We'll average the two weights for intersection
        }
    }

    // Weighted Jaccard-like similarity
    let union_weighted = source_weighted_count + target_weighted_count - intersection_weighted;
    if union_weighted > 0.0 {
        intersection_weighted / union_weighted
    } else {
        0.0
    }
}

/// Gets the set of column names that are part of the primary key.
fn get_primary_key_columns(table: &Table) -> HashSet<&str> {
    let mut pk_cols = HashSet::new();
    for constraint in &table.constraints {
        if let ConstraintKind::PrimaryKey(pk) = &constraint.kind {
            for col in &pk.columns {
                pk_cols.insert(col.as_ref());
            }
        }
    }
    pk_cols
}

/// Gets the set of column names that are part of foreign keys.
fn get_foreign_key_columns(table: &Table) -> HashSet<&str> {
    let mut fk_cols = HashSet::new();
    for constraint in &table.constraints {
        if let ConstraintKind::ForeignKey(fk) = &constraint.kind {
            for col in &fk.columns {
                fk_cols.insert(col.as_ref());
            }
        }
    }
    fk_cols
}

/// Calculates type match ratio for columns with matching names.
fn type_match_similarity(source: &Table, target: &Table) -> f64 {
    let source_cols: std::collections::HashMap<&str, &Column> = source
        .columns
        .iter()
        .map(|c| (c.name.as_ref(), c))
        .collect();

    let mut matches = 0;
    let mut total = 0;

    for target_col in &target.columns {
        if let Some(source_col) = source_cols.get(target_col.name.as_ref()) {
            total += 1;
            if source_col.type_info == target_col.type_info {
                matches += 1;
            }
        }
    }

    if total > 0 {
        matches as f64 / total as f64
    } else {
        0.0
    }
}

/// Calculates constraint structure similarity.
fn constraint_structure_similarity(source: &Table, target: &Table) -> f64 {
    if source.constraints.is_empty() && target.constraints.is_empty() {
        return 1.0; // Both have no constraints
    }

    if source.constraints.is_empty() || target.constraints.is_empty() {
        return 0.0;
    }

    let mut score = 0.0;
    let mut weight = 0.0;

    // Compare primary keys (most important)
    let source_pk = source
        .constraints
        .iter()
        .find(|c| matches!(c.kind, ConstraintKind::PrimaryKey(_)));
    let target_pk = target
        .constraints
        .iter()
        .find(|c| matches!(c.kind, ConstraintKind::PrimaryKey(_)));

    weight += 0.5;
    if let (Some(spk), Some(tpk)) = (source_pk, target_pk) {
        if let (ConstraintKind::PrimaryKey(s), ConstraintKind::PrimaryKey(t)) =
            (&spk.kind, &tpk.kind)
        {
            score += 0.5 * column_list_jaccard(&s.columns, &t.columns);
        }
    } else if source_pk.is_none() && target_pk.is_none() {
        score += 0.5; // Both have no PK
    }

    // Compare unique constraints
    let source_uniques: Vec<_> = source
        .constraints
        .iter()
        .filter_map(|c| {
            if let ConstraintKind::Unique(u) = &c.kind {
                Some(u)
            } else {
                None
            }
        })
        .collect();
    let target_uniques: Vec<_> = target
        .constraints
        .iter()
        .filter_map(|c| {
            if let ConstraintKind::Unique(u) = &c.kind {
                Some(u)
            } else {
                None
            }
        })
        .collect();

    if !source_uniques.is_empty() || !target_uniques.is_empty() {
        weight += 0.3;
        let unique_sim = constraint_list_similarity(&source_uniques, &target_uniques);
        score += 0.3 * unique_sim;
    }

    // Compare foreign keys
    let source_fks: Vec<_> = source
        .constraints
        .iter()
        .filter_map(|c| {
            if let ConstraintKind::ForeignKey(fk) = &c.kind {
                Some(fk)
            } else {
                None
            }
        })
        .collect();
    let target_fks: Vec<_> = target
        .constraints
        .iter()
        .filter_map(|c| {
            if let ConstraintKind::ForeignKey(fk) = &c.kind {
                Some(fk)
            } else {
                None
            }
        })
        .collect();

    if !source_fks.is_empty() || !target_fks.is_empty() {
        weight += 0.2;
        let fk_sim = foreign_key_list_similarity(&source_fks, &target_fks);
        score += 0.2 * fk_sim;
    }

    if weight > 0.0 {
        score / weight
    } else {
        0.5 // No constraints to compare
    }
}

fn column_list_jaccard(
    a: &[crate::db::schema::ColumnName],
    b: &[crate::db::schema::ColumnName],
) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let a_set: HashSet<&str> = a.iter().map(|c| c.as_ref()).collect();
    let b_set: HashSet<&str> = b.iter().map(|c| c.as_ref()).collect();

    let intersection = a_set.intersection(&b_set).count();
    let union = a_set.union(&b_set).count();

    intersection as f64 / union as f64
}

fn constraint_list_similarity(
    source: &[&crate::db::model::constraint::UniqueConstraint],
    target: &[&crate::db::model::constraint::UniqueConstraint],
) -> f64 {
    if source.is_empty() && target.is_empty() {
        return 1.0;
    }
    if source.is_empty() || target.is_empty() {
        return 0.0;
    }

    // Find best matches between source and target unique constraints
    let mut total_sim = 0.0;
    for su in source {
        let best_match = target
            .iter()
            .map(|tu| column_list_jaccard(&su.columns, &tu.columns))
            .fold(0.0_f64, |a, b| a.max(b));
        total_sim += best_match;
    }

    total_sim / source.len() as f64
}

fn foreign_key_list_similarity(
    source: &[&crate::db::model::constraint::ForeignKeyConstraint],
    target: &[&crate::db::model::constraint::ForeignKeyConstraint],
) -> f64 {
    if source.is_empty() && target.is_empty() {
        return 1.0;
    }
    if source.is_empty() || target.is_empty() {
        return 0.0;
    }

    let mut total_sim = 0.0;
    for sfk in source {
        let best_match = target
            .iter()
            .map(|tfk| {
                let col_sim = column_list_jaccard(&sfk.columns, &tfk.columns);
                let ref_sim = if sfk.referenced_table == tfk.referenced_table {
                    1.0
                } else {
                    0.0
                };
                col_sim * 0.5 + ref_sim * 0.5
            })
            .fold(0.0_f64, |a, b| a.max(b));
        total_sim += best_match;
    }

    total_sim / source.len() as f64
}

/// Calculates index structure similarity.
fn index_structure_similarity(source: &Table, target: &Table) -> f64 {
    // Filter out constraint-backing indexes
    let source_indexes: Vec<_> = source
        .indexes
        .iter()
        .filter(|i| !i.is_constraint_index)
        .collect();
    let target_indexes: Vec<_> = target
        .indexes
        .iter()
        .filter(|i| !i.is_constraint_index)
        .collect();

    if source_indexes.is_empty() && target_indexes.is_empty() {
        return 1.0;
    }
    if source_indexes.is_empty() || target_indexes.is_empty() {
        return 0.0;
    }

    let mut total_sim = 0.0;
    for si in &source_indexes {
        let best_match = target_indexes
            .iter()
            .map(|ti| single_index_similarity(si, ti))
            .fold(0.0_f64, |a, b| a.max(b));
        total_sim += best_match;
    }

    total_sim / source_indexes.len() as f64
}

fn single_index_similarity(source: &Index, target: &Index) -> f64 {
    let mut score = 0.0;

    // Method match (20%)
    if source.method == target.method {
        score += 0.2;
    }

    // Uniqueness match (15%)
    if source.is_unique == target.is_unique {
        score += 0.15;
    }

    // Column overlap (50%)
    let source_cols: Vec<&str> = source
        .columns
        .iter()
        .filter_map(|c| c.column.as_ref().map(|col| col.as_ref()))
        .collect();
    let target_cols: Vec<&str> = target
        .columns
        .iter()
        .filter_map(|c| c.column.as_ref().map(|col| col.as_ref()))
        .collect();

    if !source_cols.is_empty() || !target_cols.is_empty() {
        let source_set: HashSet<&str> = source_cols.into_iter().collect();
        let target_set: HashSet<&str> = target_cols.into_iter().collect();
        let intersection = source_set.intersection(&target_set).count();
        let union = source_set.union(&target_set).count();
        if union > 0 {
            score += 0.5 * (intersection as f64 / union as f64);
        }
    } else {
        score += 0.5; // Both are expression indexes
    }

    // Predicate match (15%)
    if source.predicate == target.predicate {
        score += 0.15;
    }

    score
}

// =============================================================================
// Column Similarity
// =============================================================================

/// Configuration for column rename detection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnSimilarityConfig {
    /// Weight for type match (most important).
    /// Range: 0.0-1.0, default: 0.50
    pub type_weight: f64,

    /// Weight for position proximity.
    /// Range: 0.0-1.0, default: 0.15
    pub position_weight: f64,

    /// Weight for nullability match.
    /// Range: 0.0-1.0, default: 0.10
    pub nullability_weight: f64,

    /// Weight for default value match.
    /// Range: 0.0-1.0, default: 0.10
    pub default_weight: f64,

    /// Weight for name similarity.
    /// Range: 0.0-1.0, default: 0.15
    pub name_weight: f64,

    /// If types don't match, cap similarity at this value.
    /// Range: 0.0-0.5, default: 0.35
    pub type_mismatch_cap: f64,
}

impl Default for ColumnSimilarityConfig {
    fn default() -> Self {
        Self {
            type_weight: 0.50,
            position_weight: 0.15,
            nullability_weight: 0.10,
            default_weight: 0.10,
            name_weight: 0.15,
            type_mismatch_cap: 0.35,
        }
    }
}

/// Calculates similarity between two columns (0.0 to 1.0).
pub fn column_similarity(source: &Column, target: &Column, config: &ColumnSimilarityConfig) -> f64 {
    let type_matches = source.type_info == target.type_info;

    // If types don't match, cap the maximum similarity
    if !type_matches {
        let pos_diff = (source.position - target.position).abs() as f64;
        let pos_score = 1.0 / (1.0 + pos_diff * 0.5);
        let name_sim = name_similarity(source.name.as_ref(), target.name.as_ref());

        // Even with type mismatch, give some credit for name similarity
        return config.type_mismatch_cap * (pos_score * 0.3 + name_sim * 0.7);
    }

    // Types match: calculate full similarity
    let total_weight = config.type_weight
        + config.position_weight
        + config.nullability_weight
        + config.default_weight
        + config.name_weight;

    let mut score = config.type_weight; // Base score for type match

    // Nullability match
    if source.is_nullable == target.is_nullable {
        score += config.nullability_weight;
    }

    // Default match
    if source.default == target.default {
        score += config.default_weight;
    }

    // Position proximity
    let pos_diff = (source.position - target.position).abs() as f64;
    let pos_score = 1.0 / (1.0 + pos_diff * 0.2);
    score += config.position_weight * pos_score;

    // Name similarity
    let name_sim = name_similarity(source.name.as_ref(), target.name.as_ref());
    score += config.name_weight * name_sim;

    score / total_weight
}

#[cfg(test)]
mod tests {
    use super::*;

    mod levenshtein_tests {
        use super::*;

        #[test]
        fn identical_strings() {
            assert_eq!(levenshtein_distance("hello", "hello"), 0);
        }

        #[test]
        fn empty_strings() {
            assert_eq!(levenshtein_distance("", ""), 0);
            assert_eq!(levenshtein_distance("abc", ""), 3);
            assert_eq!(levenshtein_distance("", "abc"), 3);
        }

        #[test]
        fn single_edit() {
            assert_eq!(levenshtein_distance("cat", "car"), 1); // substitution
            assert_eq!(levenshtein_distance("cat", "cats"), 1); // insertion
            assert_eq!(levenshtein_distance("cats", "cat"), 1); // deletion
        }

        #[test]
        fn multiple_edits() {
            assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
            assert_eq!(levenshtein_distance("saturday", "sunday"), 3);
        }
    }

    mod name_similarity_tests {
        use super::*;

        #[test]
        fn identical_names() {
            assert_eq!(name_similarity("users", "users"), 1.0);
        }

        #[test]
        fn case_insensitive() {
            let sim = name_similarity("Users", "users");
            assert!(sim > 0.95);
        }

        #[test]
        fn similar_names() {
            // user -> users (small edit)
            let sim = name_similarity("user", "users");
            assert!(sim > 0.7, "user vs users: {}", sim);

            // users -> user_accounts (partial overlap via common prefix)
            let sim = name_similarity("users", "user_accounts");
            assert!(sim > 0.25, "users vs user_accounts: {}", sim);
        }

        #[test]
        fn common_prefix() {
            let sim = name_similarity("user_profiles", "user_accounts");
            // Common "user_" prefix and "user" word overlap
            assert!(sim > 0.30, "user_profiles vs user_accounts: {}", sim);
        }

        #[test]
        fn common_suffix() {
            let sim = name_similarity("user_accounts", "customer_accounts");
            assert!(sim > 0.4); // Common "_accounts" suffix
        }

        #[test]
        fn completely_different() {
            let sim = name_similarity("users", "products");
            assert!(sim < 0.4);
        }

        #[test]
        fn word_overlap() {
            let sim = name_similarity("user_email_addresses", "email_addresses");
            assert!(sim > 0.5); // Shares "email" and "addresses"
        }
    }

    mod config_tests {
        use super::*;

        #[test]
        fn default_weights_sum_to_one() {
            let config = SimilarityConfig::default();
            let total = config.column_name_weight
                + config.type_match_weight
                + config.constraint_weight
                + config.index_weight
                + config.name_similarity_weight;
            assert!((total - 1.0).abs() < 0.001);
        }

        #[test]
        fn normalized_weights() {
            let config = SimilarityConfig {
                column_name_weight: 2.0,
                type_match_weight: 2.0,
                constraint_weight: 2.0,
                index_weight: 2.0,
                name_similarity_weight: 2.0,
                ..Default::default()
            };
            let (a, b, c, d, e) = config.normalized_weights();
            assert!((a + b + c + d + e - 1.0).abs() < 0.001);
        }
    }

    mod column_similarity_tests {
        use super::*;
        use crate::db::model::types::{QualifiedCollationName, TypeInfo};
        use crate::db::schema::{CollationName, ColumnName, SchemaName, TypeName};

        fn make_column(name: &str, type_name: &str, position: i16) -> Column {
            Column {
                name: ColumnName::try_new(name.to_string()).unwrap(),
                position,
                type_info: TypeInfo {
                    name: TypeName::try_new(type_name.to_string()).unwrap(),
                    schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                    formatted: type_name.to_string(),
                    is_array: false,
                },
                is_nullable: true,
                default: None,
                generated: None,
                identity: None,
                collation: QualifiedCollationName::new(
                    SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                    CollationName::try_new("default".to_string()).unwrap(),
                ),
                comment: None,
            }
        }

        #[test]
        fn same_type_high_similarity() {
            let config = ColumnSimilarityConfig::default();
            let a = make_column("email", "text", 1);
            let b = make_column("email_address", "text", 1);

            let sim = column_similarity(&a, &b, &config);
            assert!(
                sim > 0.7,
                "Same type columns should have high similarity: {}",
                sim
            );
        }

        #[test]
        fn different_type_low_similarity() {
            let config = ColumnSimilarityConfig::default();
            let a = make_column("code", "integer", 1);
            let b = make_column("code_name", "text", 1);

            let sim = column_similarity(&a, &b, &config);
            assert!(
                sim < 0.4,
                "Different type columns should have low similarity: {}",
                sim
            );
        }

        #[test]
        fn position_affects_similarity() {
            let config = ColumnSimilarityConfig::default();
            let a = make_column("field", "text", 1);
            let b_near = make_column("field_new", "text", 2);
            let b_far = make_column("field_new", "text", 10);

            let sim_near = column_similarity(&a, &b_near, &config);
            let sim_far = column_similarity(&a, &b_far, &config);

            assert!(
                sim_near > sim_far,
                "Closer position should have higher similarity"
            );
        }
    }
}
