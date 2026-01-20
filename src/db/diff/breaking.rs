//! Breaking change detection for schema diffs.
//!
//! This module provides functionality for analyzing schema diffs to identify
//! breaking changes that would disrupt running applications. Breaking changes
//! require careful migration strategies to avoid downtime.
//!
//! # Overview
//!
//! Database migrations can be categorized by their impact on running applications:
//!
//! - **Breaking**: Changes that will cause application errors (e.g., dropping a column)
//! - **Warning**: Changes that might cause issues depending on data/usage patterns
//! - **Non-breaking**: Safe changes that won't affect running applications
//!
//! # Example
//!
//! ```
//! use tern::db::diff::{diff_namespaces, NamespaceDiff};
//! use tern::db::diff::breaking::{analyze_breaking_changes, ChangeSeverity};
//! use tern::db::model::Namespace;
//!
//! # fn example(source: Namespace, target: Namespace) {
//! let diff = diff_namespaces(&source, &target);
//! let analysis = analyze_breaking_changes(&diff);
//!
//! if analysis.has_breaking_changes() {
//!     println!("Found {} breaking changes:", analysis.breaking_changes().count());
//!     for change in analysis.breaking_changes() {
//!         println!("  - {}", change.description);
//!     }
//! }
//! # }
//! ```
//!
//! # Breaking Change Categories
//!
//! ## Definite Breaking Changes
//!
//! These operations will cause immediate failures in running applications:
//!
//! | Operation | Why It Breaks |
//! |-----------|---------------|
//! | Drop table | Queries referencing the table fail |
//! | Rename table | Queries using the old name fail |
//! | Drop column | Queries selecting/inserting the column fail |
//! | Rename column | Queries using the old column name fail |
//! | Change column type (incompatible) | Type mismatches, data truncation |
//! | Make column non-nullable | Inserts without the column fail |
//! | Remove enum value | Rows with that value become invalid |
//! | Reorder enum values | Comparison semantics change |
//!
//! ## Warning-Level Changes
//!
//! These operations might cause issues depending on data and usage:
//!
//! | Operation | Potential Issue |
//! |-----------|-----------------|
//! | Add foreign key | Existing data might violate constraint |
//! | Add check constraint | Existing data might violate constraint |
//! | Add unique constraint | Existing data might have duplicates |
//! | Change view definition | Dependent queries might break |
//!
//! ## Non-Breaking Changes
//!
//! These operations are safe for running applications:
//!
//! - Adding new tables, columns (nullable), views, sequences
//! - Making columns nullable
//! - Dropping constraints (except FK in some cases)
//! - Adding/dropping indexes (performance impact only)
//! - Adding enum values

use serde::{Deserialize, Serialize};

use crate::db::model::Constraint;
use crate::db::model::constraint::ConstraintKind;
use crate::db::model::types::{QualifiedTableName, SqlExpr, TypeInfo};
use crate::db::schema::{
    ColumnName, ConstraintName, SchemaName, SequenceName, TableName, TypeName,
};

use super::schema_diff::{ModifiedColumn, ModifiedTable, NamespaceDiff};

// =============================================================================
// Severity Classification
// =============================================================================

/// Severity level of a schema change.
///
/// Changes are classified by their potential impact on running applications.
/// This classification helps teams make informed decisions about migration
/// strategies and deployment timing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSeverity {
    /// Safe change that won't affect running applications.
    ///
    /// Examples: adding nullable columns, creating new tables, adding indexes.
    NonBreaking,

    /// Change that might cause issues depending on data or usage patterns.
    ///
    /// Examples: adding constraints that existing data might violate.
    Warning,

    /// Change that will definitely break running applications.
    ///
    /// Examples: dropping tables, renaming columns, removing enum values.
    Breaking,
}

impl ChangeSeverity {
    /// Returns true if this severity level indicates a breaking change.
    pub fn is_breaking(&self) -> bool {
        matches!(self, Self::Breaking)
    }

    /// Returns true if this severity level indicates a warning or breaking change.
    pub fn is_warning_or_worse(&self) -> bool {
        matches!(self, Self::Warning | Self::Breaking)
    }
}

// =============================================================================
// Breaking Change Kinds
// =============================================================================

/// The specific kind of breaking change detected.
///
/// Each variant captures the context needed to understand and potentially
/// mitigate the breaking change.
///
/// Note: This type implements `PartialEq` but not `Eq` because similarity
/// scores are stored as `f64` which doesn't implement `Eq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BreakingChangeKind {
    // -------------------------------------------------------------------------
    // Table-level changes
    // -------------------------------------------------------------------------
    /// A table was dropped.
    TableDropped {
        /// The name of the dropped table.
        table: TableName,
    },

    /// A table was renamed (detected via similarity matching).
    TableRenamed {
        /// The original table name.
        from: TableName,
        /// The new table name.
        to: TableName,
        /// Similarity score that triggered rename detection.
        similarity: f64,
    },

    // -------------------------------------------------------------------------
    // Column-level changes
    // -------------------------------------------------------------------------
    /// A column was dropped from a table.
    ColumnDropped {
        /// The table containing the column.
        table: TableName,
        /// The name of the dropped column.
        column: ColumnName,
    },

    /// A column was renamed (detected via similarity matching).
    ColumnRenamed {
        /// The table containing the column.
        table: TableName,
        /// The original column name.
        from: ColumnName,
        /// The new column name.
        to: ColumnName,
        /// Similarity score that triggered rename detection.
        similarity: f64,
    },

    /// A column's type was changed in a potentially incompatible way.
    ColumnTypeChanged {
        /// The table containing the column.
        table: TableName,
        /// The column name.
        column: ColumnName,
        /// The original type.
        from_type: TypeInfo,
        /// The new type.
        to_type: TypeInfo,
    },

    /// A column was changed from nullable to non-nullable.
    ColumnMadeNonNullable {
        /// The table containing the column.
        table: TableName,
        /// The column name.
        column: ColumnName,
    },

    // -------------------------------------------------------------------------
    // Constraint-level changes
    // -------------------------------------------------------------------------
    /// A primary key constraint was added to an existing table.
    PrimaryKeyAdded {
        /// The table receiving the constraint.
        table: TableName,
        /// The constraint name.
        constraint: ConstraintName,
        /// The columns in the primary key.
        columns: Vec<ColumnName>,
    },

    /// A unique constraint was added to an existing table.
    UniqueConstraintAdded {
        /// The table receiving the constraint.
        table: TableName,
        /// The constraint name.
        constraint: ConstraintName,
        /// The columns in the unique constraint.
        columns: Vec<ColumnName>,
    },

    /// A check constraint was added to an existing table.
    CheckConstraintAdded {
        /// The table receiving the constraint.
        table: TableName,
        /// The constraint name.
        constraint: ConstraintName,
        /// The check expression.
        expression: SqlExpr,
    },

    /// A foreign key constraint was added to an existing table.
    ForeignKeyAdded {
        /// The table receiving the constraint.
        table: TableName,
        /// The constraint name.
        constraint: ConstraintName,
        /// The columns in the foreign key.
        columns: Vec<ColumnName>,
        /// The referenced table.
        referenced_table: QualifiedTableName,
    },

    /// An exclusion constraint was added to an existing table.
    ExclusionConstraintAdded {
        /// The table receiving the constraint.
        table: TableName,
        /// The constraint name.
        constraint: ConstraintName,
    },

    // -------------------------------------------------------------------------
    // Enum-level changes
    // -------------------------------------------------------------------------
    /// Values were removed from an enum type.
    EnumValueRemoved {
        /// The enum type name.
        enum_type: TypeName,
        /// The values that were removed.
        values: Vec<String>,
    },

    /// Enum values were reordered.
    ///
    /// PostgreSQL doesn't support reordering enum values, so this requires
    /// recreating the enum type, which is a complex migration.
    EnumValuesReordered {
        /// The enum type name.
        enum_type: TypeName,
    },

    // -------------------------------------------------------------------------
    // View-level changes
    // -------------------------------------------------------------------------
    /// A view was dropped.
    ViewDropped {
        /// The name of the dropped view.
        view: TableName,
    },

    /// A view was renamed (detected via similarity matching).
    ViewRenamed {
        /// The original view name.
        from: TableName,
        /// The new view name.
        to: TableName,
        /// Similarity score that triggered rename detection.
        similarity: f64,
    },

    /// A view's materialization status changed.
    MaterializationChanged {
        /// The view name.
        view: TableName,
        /// True if the view became materialized, false if it became regular.
        became_materialized: bool,
    },

    // -------------------------------------------------------------------------
    // Sequence-level changes
    // -------------------------------------------------------------------------
    /// A sequence was dropped.
    SequenceDropped {
        /// The name of the dropped sequence.
        sequence: SequenceName,
    },

    /// A sequence was renamed (detected via similarity matching).
    SequenceRenamed {
        /// The original sequence name.
        from: SequenceName,
        /// The new sequence name.
        to: SequenceName,
        /// Similarity score that triggered rename detection.
        similarity: f64,
    },
}

impl BreakingChangeKind {
    /// Returns the default severity for this kind of change.
    ///
    /// Most changes have a fixed severity, but some (like constraint additions)
    /// are classified as warnings since they might succeed depending on data.
    pub fn default_severity(&self) -> ChangeSeverity {
        match self {
            // Definite breaking changes
            Self::TableDropped { .. }
            | Self::TableRenamed { .. }
            | Self::ColumnDropped { .. }
            | Self::ColumnRenamed { .. }
            | Self::ColumnMadeNonNullable { .. }
            | Self::EnumValueRemoved { .. }
            | Self::EnumValuesReordered { .. }
            | Self::ViewDropped { .. }
            | Self::ViewRenamed { .. }
            | Self::MaterializationChanged { .. }
            | Self::SequenceDropped { .. }
            | Self::SequenceRenamed { .. } => ChangeSeverity::Breaking,

            // Type changes need analysis - default to breaking for safety
            Self::ColumnTypeChanged { .. } => ChangeSeverity::Breaking,

            // Constraint additions might fail on existing data
            Self::PrimaryKeyAdded { .. }
            | Self::UniqueConstraintAdded { .. }
            | Self::CheckConstraintAdded { .. }
            | Self::ForeignKeyAdded { .. }
            | Self::ExclusionConstraintAdded { .. } => ChangeSeverity::Warning,
        }
    }
}

// =============================================================================
// Breaking Change
// =============================================================================

/// A detected breaking change with full context.
///
/// Contains the specific change kind, its severity, and a human-readable description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakingChange {
    /// The specific kind of breaking change.
    pub kind: BreakingChangeKind,
    /// The severity of this change.
    pub severity: ChangeSeverity,
    /// Human-readable description of the change.
    pub description: String,
}

impl BreakingChange {
    /// Creates a new breaking change from a kind, using the default severity.
    pub fn new(kind: BreakingChangeKind) -> Self {
        let severity = kind.default_severity();
        let description = Self::describe(&kind);
        Self {
            kind,
            severity,
            description,
        }
    }

    /// Creates a new breaking change with a custom severity.
    pub fn with_severity(kind: BreakingChangeKind, severity: ChangeSeverity) -> Self {
        let description = Self::describe(&kind);
        Self {
            kind,
            severity,
            description,
        }
    }

    /// Generates a human-readable description for a breaking change kind.
    fn describe(kind: &BreakingChangeKind) -> String {
        match kind {
            BreakingChangeKind::TableDropped { table } => {
                format!("Table '{}' was dropped", table.as_ref())
            }
            BreakingChangeKind::TableRenamed {
                from,
                to,
                similarity,
            } => {
                format!(
                    "Table '{}' was renamed to '{}' (similarity: {:.0}%)",
                    from.as_ref(),
                    to.as_ref(),
                    similarity * 100.0
                )
            }
            BreakingChangeKind::ColumnDropped { table, column } => {
                format!(
                    "Column '{}.{}' was dropped",
                    table.as_ref(),
                    column.as_ref()
                )
            }
            BreakingChangeKind::ColumnRenamed {
                table,
                from,
                to,
                similarity,
            } => {
                format!(
                    "Column '{}.{}' was renamed to '{}' (similarity: {:.0}%)",
                    table.as_ref(),
                    from.as_ref(),
                    to.as_ref(),
                    similarity * 100.0
                )
            }
            BreakingChangeKind::ColumnTypeChanged {
                table,
                column,
                from_type,
                to_type,
            } => {
                format!(
                    "Column '{}.{}' type changed from '{}' to '{}'",
                    table.as_ref(),
                    column.as_ref(),
                    from_type.formatted,
                    to_type.formatted
                )
            }
            BreakingChangeKind::ColumnMadeNonNullable { table, column } => {
                format!(
                    "Column '{}.{}' changed from nullable to NOT NULL",
                    table.as_ref(),
                    column.as_ref()
                )
            }
            BreakingChangeKind::PrimaryKeyAdded {
                table,
                constraint,
                columns,
            } => {
                let cols: Vec<&str> = columns.iter().map(|c| c.as_ref()).collect();
                format!(
                    "Primary key '{}' added to table '{}' on columns ({})",
                    constraint.as_ref(),
                    table.as_ref(),
                    cols.join(", ")
                )
            }
            BreakingChangeKind::UniqueConstraintAdded {
                table,
                constraint,
                columns,
            } => {
                let cols: Vec<&str> = columns.iter().map(|c| c.as_ref()).collect();
                format!(
                    "Unique constraint '{}' added to table '{}' on columns ({})",
                    constraint.as_ref(),
                    table.as_ref(),
                    cols.join(", ")
                )
            }
            BreakingChangeKind::CheckConstraintAdded {
                table,
                constraint,
                expression,
            } => {
                format!(
                    "Check constraint '{}' added to table '{}': {}",
                    constraint.as_ref(),
                    table.as_ref(),
                    expression.as_ref()
                )
            }
            BreakingChangeKind::ForeignKeyAdded {
                table,
                constraint,
                columns,
                referenced_table,
            } => {
                let cols: Vec<&str> = columns.iter().map(|c| c.as_ref()).collect();
                format!(
                    "Foreign key '{}' added to table '{}' ({}) referencing '{}'",
                    constraint.as_ref(),
                    table.as_ref(),
                    cols.join(", "),
                    referenced_table
                )
            }
            BreakingChangeKind::ExclusionConstraintAdded { table, constraint } => {
                format!(
                    "Exclusion constraint '{}' added to table '{}'",
                    constraint.as_ref(),
                    table.as_ref()
                )
            }
            BreakingChangeKind::EnumValueRemoved { enum_type, values } => {
                format!(
                    "Values removed from enum '{}': {}",
                    enum_type.as_ref(),
                    values.join(", ")
                )
            }
            BreakingChangeKind::EnumValuesReordered { enum_type } => {
                format!("Enum '{}' values were reordered", enum_type.as_ref())
            }
            BreakingChangeKind::ViewDropped { view } => {
                format!("View '{}' was dropped", view.as_ref())
            }
            BreakingChangeKind::ViewRenamed {
                from,
                to,
                similarity,
            } => {
                format!(
                    "View '{}' was renamed to '{}' (similarity: {:.0}%)",
                    from.as_ref(),
                    to.as_ref(),
                    similarity * 100.0
                )
            }
            BreakingChangeKind::MaterializationChanged {
                view,
                became_materialized,
            } => {
                if *became_materialized {
                    format!(
                        "View '{}' changed from regular to materialized",
                        view.as_ref()
                    )
                } else {
                    format!(
                        "View '{}' changed from materialized to regular",
                        view.as_ref()
                    )
                }
            }
            BreakingChangeKind::SequenceDropped { sequence } => {
                format!("Sequence '{}' was dropped", sequence.as_ref())
            }
            BreakingChangeKind::SequenceRenamed {
                from,
                to,
                similarity,
            } => {
                format!(
                    "Sequence '{}' was renamed to '{}' (similarity: {:.0}%)",
                    from.as_ref(),
                    to.as_ref(),
                    similarity * 100.0
                )
            }
        }
    }
}

// =============================================================================
// Analysis Result
// =============================================================================

/// Result of analyzing a schema diff for breaking changes.
///
/// Contains all detected changes categorized by severity, along with
/// summary statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakingChangeAnalysis {
    /// The schema that was analyzed.
    pub schema: SchemaName,
    /// All detected breaking or warning-level changes.
    changes: Vec<BreakingChange>,
}

impl BreakingChangeAnalysis {
    /// Creates an empty analysis for a schema.
    pub fn new(schema: SchemaName) -> Self {
        Self {
            schema,
            changes: Vec::new(),
        }
    }

    /// Adds a breaking change to the analysis.
    pub fn add(&mut self, change: BreakingChange) {
        self.changes.push(change);
    }

    /// Returns true if there are any breaking changes.
    pub fn has_breaking_changes(&self) -> bool {
        self.changes.iter().any(|c| c.severity.is_breaking())
    }

    /// Returns true if there are any warnings or breaking changes.
    pub fn has_warnings_or_breaking(&self) -> bool {
        self.changes
            .iter()
            .any(|c| c.severity.is_warning_or_worse())
    }

    /// Returns true if there are no breaking changes or warnings.
    pub fn is_safe(&self) -> bool {
        self.changes.is_empty()
    }

    /// Returns an iterator over all changes.
    pub fn all_changes(&self) -> impl Iterator<Item = &BreakingChange> {
        self.changes.iter()
    }

    /// Returns an iterator over breaking changes only.
    pub fn breaking_changes(&self) -> impl Iterator<Item = &BreakingChange> {
        self.changes
            .iter()
            .filter(|c| c.severity == ChangeSeverity::Breaking)
    }

    /// Returns an iterator over warning-level changes only.
    pub fn warnings(&self) -> impl Iterator<Item = &BreakingChange> {
        self.changes
            .iter()
            .filter(|c| c.severity == ChangeSeverity::Warning)
    }

    /// Returns the count of breaking changes.
    pub fn breaking_count(&self) -> usize {
        self.changes
            .iter()
            .filter(|c| c.severity == ChangeSeverity::Breaking)
            .count()
    }

    /// Returns the count of warnings.
    pub fn warning_count(&self) -> usize {
        self.changes
            .iter()
            .filter(|c| c.severity == ChangeSeverity::Warning)
            .count()
    }

    /// Returns the total count of all changes.
    pub fn total_count(&self) -> usize {
        self.changes.len()
    }

    /// Consumes the analysis and returns the underlying changes.
    pub fn into_changes(self) -> Vec<BreakingChange> {
        self.changes
    }
}

// =============================================================================
// Analysis Functions
// =============================================================================

/// Analyzes a namespace diff for breaking changes.
///
/// Examines all changes in the diff and classifies them by severity.
/// Returns an analysis containing all breaking changes and warnings.
///
/// # Example
///
/// ```
/// use tern::db::diff::{diff_namespaces, NamespaceDiff};
/// use tern::db::diff::breaking::analyze_breaking_changes;
/// use tern::db::model::Namespace;
///
/// # fn example(source: Namespace, target: Namespace) {
/// let diff = diff_namespaces(&source, &target);
/// let analysis = analyze_breaking_changes(&diff);
///
/// println!("Breaking changes: {}", analysis.breaking_count());
/// println!("Warnings: {}", analysis.warning_count());
/// # }
/// ```
pub fn analyze_breaking_changes(diff: &NamespaceDiff) -> BreakingChangeAnalysis {
    let mut analysis = BreakingChangeAnalysis::new(diff.name.clone());

    // Analyze tables
    analyze_table_changes(diff, &mut analysis);

    // Analyze views
    analyze_view_changes(diff, &mut analysis);

    // Analyze sequences
    analyze_sequence_changes(diff, &mut analysis);

    // Analyze enums
    analyze_enum_changes(diff, &mut analysis);

    analysis
}

/// Analyzes table-level changes for breaking changes.
fn analyze_table_changes(diff: &NamespaceDiff, analysis: &mut BreakingChangeAnalysis) {
    // Dropped tables are breaking
    for table_name in &diff.tables.removed {
        analysis.add(BreakingChange::new(BreakingChangeKind::TableDropped {
            table: table_name.clone(),
        }));
    }

    // Renamed tables are breaking
    for rename in &diff.tables.potential_renames {
        analysis.add(BreakingChange::new(BreakingChangeKind::TableRenamed {
            from: rename.source_key.clone(),
            to: rename.target.name.clone(),
            similarity: rename.similarity,
        }));
    }

    // Analyze modifications to existing tables
    for modified_table in &diff.tables.modified {
        analyze_modified_table(modified_table, analysis);
    }
}

/// Analyzes a modified table for breaking changes.
fn analyze_modified_table(table: &ModifiedTable, analysis: &mut BreakingChangeAnalysis) {
    let table_name = &table.name;

    // Dropped columns are breaking
    for column_name in &table.columns.removed {
        analysis.add(BreakingChange::new(BreakingChangeKind::ColumnDropped {
            table: table_name.clone(),
            column: column_name.clone(),
        }));
    }

    // Renamed columns are breaking
    for rename in &table.columns.potential_renames {
        analysis.add(BreakingChange::new(BreakingChangeKind::ColumnRenamed {
            table: table_name.clone(),
            from: rename.source_key.clone(),
            to: rename.target.name.clone(),
            similarity: rename.similarity,
        }));
    }

    // Analyze column modifications
    for modified_column in &table.columns.modified {
        analyze_modified_column(table_name, modified_column, analysis);
    }

    // Analyze added constraints
    for constraint in &table.constraints.added {
        analyze_added_constraint(table_name, constraint, analysis);
    }
}

/// Analyzes a modified column for breaking changes.
fn analyze_modified_column(
    table_name: &TableName,
    column: &ModifiedColumn,
    analysis: &mut BreakingChangeAnalysis,
) {
    // Type changes are potentially breaking
    if let Some(type_change) = &column.type_info {
        let severity = classify_type_change(&type_change.source, &type_change.target);
        if severity.is_warning_or_worse() {
            analysis.add(BreakingChange::with_severity(
                BreakingChangeKind::ColumnTypeChanged {
                    table: table_name.clone(),
                    column: column.name.clone(),
                    from_type: type_change.source.clone(),
                    to_type: type_change.target.clone(),
                },
                severity,
            ));
        }
    }

    // Nullable to non-nullable is breaking
    if let Some(nullable_change) = &column.is_nullable
        && nullable_change.source
        && !nullable_change.target
    {
        analysis.add(BreakingChange::new(
            BreakingChangeKind::ColumnMadeNonNullable {
                table: table_name.clone(),
                column: column.name.clone(),
            },
        ));
    }
}

/// Analyzes an added constraint for breaking/warning classification.
fn analyze_added_constraint(
    table_name: &TableName,
    constraint: &Constraint,
    analysis: &mut BreakingChangeAnalysis,
) {
    let change_kind = match &constraint.kind {
        ConstraintKind::PrimaryKey(pk) => BreakingChangeKind::PrimaryKeyAdded {
            table: table_name.clone(),
            constraint: constraint.name.clone(),
            columns: pk.columns.clone(),
        },
        ConstraintKind::Unique(unique) => BreakingChangeKind::UniqueConstraintAdded {
            table: table_name.clone(),
            constraint: constraint.name.clone(),
            columns: unique.columns.clone(),
        },
        ConstraintKind::Check(check) => BreakingChangeKind::CheckConstraintAdded {
            table: table_name.clone(),
            constraint: constraint.name.clone(),
            expression: check.expression.clone(),
        },
        ConstraintKind::ForeignKey(fk) => BreakingChangeKind::ForeignKeyAdded {
            table: table_name.clone(),
            constraint: constraint.name.clone(),
            columns: fk.columns.clone(),
            referenced_table: fk.referenced_table.clone(),
        },
        ConstraintKind::Exclusion(_) => BreakingChangeKind::ExclusionConstraintAdded {
            table: table_name.clone(),
            constraint: constraint.name.clone(),
        },
    };

    analysis.add(BreakingChange::new(change_kind));
}

/// Analyzes view-level changes for breaking changes.
fn analyze_view_changes(diff: &NamespaceDiff, analysis: &mut BreakingChangeAnalysis) {
    // Dropped views are breaking
    for view_name in &diff.views.removed {
        analysis.add(BreakingChange::new(BreakingChangeKind::ViewDropped {
            view: view_name.clone(),
        }));
    }

    // Renamed views are breaking
    for rename in &diff.views.potential_renames {
        analysis.add(BreakingChange::new(BreakingChangeKind::ViewRenamed {
            from: rename.source_key.clone(),
            to: rename.target.name.clone(),
            similarity: rename.similarity,
        }));
    }

    // Check for materialization changes
    for modified_view in &diff.views.modified {
        if let Some(mat_change) = &modified_view.is_materialized {
            analysis.add(BreakingChange::new(
                BreakingChangeKind::MaterializationChanged {
                    view: modified_view.name.clone(),
                    became_materialized: mat_change.target,
                },
            ));
        }
    }
}

/// Analyzes sequence-level changes for breaking changes.
fn analyze_sequence_changes(diff: &NamespaceDiff, analysis: &mut BreakingChangeAnalysis) {
    // Dropped sequences are breaking
    for seq_name in &diff.sequences.removed {
        analysis.add(BreakingChange::new(BreakingChangeKind::SequenceDropped {
            sequence: seq_name.clone(),
        }));
    }

    // Renamed sequences are breaking
    for rename in &diff.sequences.potential_renames {
        analysis.add(BreakingChange::new(BreakingChangeKind::SequenceRenamed {
            from: rename.source_key.clone(),
            to: rename.target.name.clone(),
            similarity: rename.similarity,
        }));
    }
}

/// Analyzes enum-level changes for breaking changes.
fn analyze_enum_changes(diff: &NamespaceDiff, analysis: &mut BreakingChangeAnalysis) {
    // Analyze modified enums
    for modified_enum in &diff.enums.modified {
        // Removed values are breaking
        if !modified_enum.values_removed.is_empty() {
            analysis.add(BreakingChange::new(BreakingChangeKind::EnumValueRemoved {
                enum_type: modified_enum.name.clone(),
                values: modified_enum.values_removed.clone(),
            }));
        }

        // Reordered values are breaking
        if modified_enum.values_reordered {
            analysis.add(BreakingChange::new(
                BreakingChangeKind::EnumValuesReordered {
                    enum_type: modified_enum.name.clone(),
                },
            ));
        }
    }
}

// =============================================================================
// Type Change Classification
// =============================================================================

/// Classifies a column type change by its breaking potential.
///
/// This function analyzes whether a type change is:
/// - **Non-breaking**: Safe changes like widening VARCHAR or increasing precision
/// - **Warning**: Changes that might lose data depending on actual values
/// - **Breaking**: Changes that will definitely cause issues
fn classify_type_change(from: &TypeInfo, to: &TypeInfo) -> ChangeSeverity {
    // Same type with same formatting - not a real change
    if from.formatted == to.formatted {
        return ChangeSeverity::NonBreaking;
    }

    // Check for known safe widening operations
    if is_safe_type_widening(from, to) {
        return ChangeSeverity::NonBreaking;
    }

    // Check for known narrowing operations (data loss risk)
    if is_type_narrowing(from, to) {
        return ChangeSeverity::Breaking;
    }

    // Default to breaking for any other type change
    // This is conservative - better to warn than miss a breaking change
    ChangeSeverity::Breaking
}

/// Checks if a type change is a safe widening operation.
///
/// Safe widenings include:
/// - VARCHAR(n) -> VARCHAR(m) where m > n
/// - NUMERIC(p,s) -> NUMERIC(p',s') where p' >= p and s' >= s
/// - smallint -> integer -> bigint
/// - real -> double precision
fn is_safe_type_widening(from: &TypeInfo, to: &TypeInfo) -> bool {
    let from_name = from.name.as_ref();
    let to_name = to.name.as_ref();

    // Integer promotions
    if (from_name == "int2" || from_name == "smallint")
        && (to_name == "int4" || to_name == "integer" || to_name == "int8" || to_name == "bigint")
    {
        return true;
    }
    if (from_name == "int4" || from_name == "integer") && (to_name == "int8" || to_name == "bigint")
    {
        return true;
    }

    // Float promotions
    if (from_name == "float4" || from_name == "real")
        && (to_name == "float8" || to_name == "double precision")
    {
        return true;
    }

    // VARCHAR widening (requires parsing the formatted type)
    if from_name == "varchar" && to_name == "varchar" {
        if let (Some(from_len), Some(to_len)) = (
            parse_varchar_length(&from.formatted),
            parse_varchar_length(&to.formatted),
        ) {
            return to_len >= from_len;
        }
        // varchar(n) -> varchar (unlimited) is safe
        if parse_varchar_length(&from.formatted).is_some()
            && parse_varchar_length(&to.formatted).is_none()
        {
            return true;
        }
    }

    // text is wider than any varchar
    if from_name == "varchar" && to_name == "text" {
        return true;
    }

    false
}

/// Checks if a type change is a narrowing operation (potential data loss).
fn is_type_narrowing(from: &TypeInfo, to: &TypeInfo) -> bool {
    let from_name = from.name.as_ref();
    let to_name = to.name.as_ref();

    // Integer demotions
    if (from_name == "int8" || from_name == "bigint")
        && (to_name == "int4" || to_name == "integer" || to_name == "int2" || to_name == "smallint")
    {
        return true;
    }
    if (from_name == "int4" || from_name == "integer")
        && (to_name == "int2" || to_name == "smallint")
    {
        return true;
    }

    // Float demotions
    if (from_name == "float8" || from_name == "double precision")
        && (to_name == "float4" || to_name == "real")
    {
        return true;
    }

    // VARCHAR narrowing
    if from_name == "varchar" && to_name == "varchar" {
        if let (Some(from_len), Some(to_len)) = (
            parse_varchar_length(&from.formatted),
            parse_varchar_length(&to.formatted),
        ) {
            return to_len < from_len;
        }
        // varchar (unlimited) -> varchar(n) is narrowing
        if parse_varchar_length(&from.formatted).is_none()
            && parse_varchar_length(&to.formatted).is_some()
        {
            return true;
        }
    }

    // text -> varchar is narrowing
    if from_name == "text" && to_name == "varchar" {
        return true;
    }

    false
}

/// Parses the length from a VARCHAR formatted type string.
///
/// Examples:
/// - "character varying(255)" -> Some(255)
/// - "character varying" -> None (unlimited)
/// - "varchar(100)" -> Some(100)
fn parse_varchar_length(formatted: &str) -> Option<u32> {
    // Try to find a number in parentheses
    if let Some(start) = formatted.find('(')
        && let Some(end) = formatted.find(')')
        && start < end
    {
        return formatted[start + 1..end].trim().parse().ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    mod severity_tests {
        use super::*;

        #[test]
        fn severity_ordering() {
            assert!(ChangeSeverity::NonBreaking < ChangeSeverity::Warning);
            assert!(ChangeSeverity::Warning < ChangeSeverity::Breaking);
        }

        #[test]
        fn is_breaking() {
            assert!(!ChangeSeverity::NonBreaking.is_breaking());
            assert!(!ChangeSeverity::Warning.is_breaking());
            assert!(ChangeSeverity::Breaking.is_breaking());
        }

        #[test]
        fn is_warning_or_worse() {
            assert!(!ChangeSeverity::NonBreaking.is_warning_or_worse());
            assert!(ChangeSeverity::Warning.is_warning_or_worse());
            assert!(ChangeSeverity::Breaking.is_warning_or_worse());
        }
    }

    mod type_change_tests {
        use super::*;

        fn make_type_info(name: &str, formatted: &str) -> TypeInfo {
            TypeInfo {
                name: TypeName::try_new(name.to_string()).unwrap(),
                schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                formatted: formatted.to_string(),
                is_array: false,
            }
        }

        #[test]
        fn same_type_is_non_breaking() {
            let t = make_type_info("int4", "integer");
            assert_eq!(classify_type_change(&t, &t), ChangeSeverity::NonBreaking);
        }

        #[test]
        fn integer_widening_is_safe() {
            let small = make_type_info("int2", "smallint");
            let int = make_type_info("int4", "integer");
            let big = make_type_info("int8", "bigint");

            assert_eq!(
                classify_type_change(&small, &int),
                ChangeSeverity::NonBreaking
            );
            assert_eq!(
                classify_type_change(&small, &big),
                ChangeSeverity::NonBreaking
            );
            assert_eq!(
                classify_type_change(&int, &big),
                ChangeSeverity::NonBreaking
            );
        }

        #[test]
        fn integer_narrowing_is_breaking() {
            let small = make_type_info("int2", "smallint");
            let int = make_type_info("int4", "integer");
            let big = make_type_info("int8", "bigint");

            assert_eq!(classify_type_change(&big, &int), ChangeSeverity::Breaking);
            assert_eq!(classify_type_change(&big, &small), ChangeSeverity::Breaking);
            assert_eq!(classify_type_change(&int, &small), ChangeSeverity::Breaking);
        }

        #[test]
        fn varchar_widening_is_safe() {
            let v100 = make_type_info("varchar", "character varying(100)");
            let v200 = make_type_info("varchar", "character varying(200)");
            let vunlimited = make_type_info("varchar", "character varying");

            assert_eq!(
                classify_type_change(&v100, &v200),
                ChangeSeverity::NonBreaking
            );
            assert_eq!(
                classify_type_change(&v100, &vunlimited),
                ChangeSeverity::NonBreaking
            );
        }

        #[test]
        fn varchar_narrowing_is_breaking() {
            let v100 = make_type_info("varchar", "character varying(100)");
            let v200 = make_type_info("varchar", "character varying(200)");
            let vunlimited = make_type_info("varchar", "character varying");

            assert_eq!(classify_type_change(&v200, &v100), ChangeSeverity::Breaking);
            assert_eq!(
                classify_type_change(&vunlimited, &v100),
                ChangeSeverity::Breaking
            );
        }

        #[test]
        fn varchar_to_text_is_safe() {
            let v = make_type_info("varchar", "character varying(100)");
            let t = make_type_info("text", "text");

            assert_eq!(classify_type_change(&v, &t), ChangeSeverity::NonBreaking);
        }

        #[test]
        fn text_to_varchar_is_breaking() {
            let v = make_type_info("varchar", "character varying(100)");
            let t = make_type_info("text", "text");

            assert_eq!(classify_type_change(&t, &v), ChangeSeverity::Breaking);
        }

        #[test]
        fn float_widening_is_safe() {
            let real = make_type_info("float4", "real");
            let double = make_type_info("float8", "double precision");

            assert_eq!(
                classify_type_change(&real, &double),
                ChangeSeverity::NonBreaking
            );
        }

        #[test]
        fn float_narrowing_is_breaking() {
            let real = make_type_info("float4", "real");
            let double = make_type_info("float8", "double precision");

            assert_eq!(
                classify_type_change(&double, &real),
                ChangeSeverity::Breaking
            );
        }
    }

    mod varchar_parsing_tests {
        use super::*;

        #[test]
        fn parse_varchar_with_length() {
            assert_eq!(parse_varchar_length("character varying(255)"), Some(255));
            assert_eq!(parse_varchar_length("varchar(100)"), Some(100));
        }

        #[test]
        fn parse_varchar_unlimited() {
            assert_eq!(parse_varchar_length("character varying"), None);
            assert_eq!(parse_varchar_length("varchar"), None);
        }
    }

    mod analysis_tests {
        use super::*;

        #[test]
        fn empty_analysis() {
            let schema = SchemaName::try_new("public".to_string()).unwrap();
            let analysis = BreakingChangeAnalysis::new(schema);

            assert!(analysis.is_safe());
            assert!(!analysis.has_breaking_changes());
            assert!(!analysis.has_warnings_or_breaking());
            assert_eq!(analysis.total_count(), 0);
        }

        #[test]
        fn analysis_with_breaking_change() {
            let schema = SchemaName::try_new("public".to_string()).unwrap();
            let mut analysis = BreakingChangeAnalysis::new(schema);

            let table = TableName::try_new("users".to_string()).unwrap();
            analysis.add(BreakingChange::new(BreakingChangeKind::TableDropped {
                table,
            }));

            assert!(!analysis.is_safe());
            assert!(analysis.has_breaking_changes());
            assert!(analysis.has_warnings_or_breaking());
            assert_eq!(analysis.breaking_count(), 1);
            assert_eq!(analysis.warning_count(), 0);
        }

        #[test]
        fn analysis_with_warning() {
            let schema = SchemaName::try_new("public".to_string()).unwrap();
            let mut analysis = BreakingChangeAnalysis::new(schema);

            let table = TableName::try_new("users".to_string()).unwrap();
            let constraint = ConstraintName::try_new("users_email_key".to_string()).unwrap();
            let column = ColumnName::try_new("email".to_string()).unwrap();

            analysis.add(BreakingChange::new(
                BreakingChangeKind::UniqueConstraintAdded {
                    table,
                    constraint,
                    columns: vec![column],
                },
            ));

            assert!(!analysis.is_safe());
            assert!(!analysis.has_breaking_changes());
            assert!(analysis.has_warnings_or_breaking());
            assert_eq!(analysis.breaking_count(), 0);
            assert_eq!(analysis.warning_count(), 1);
        }
    }

    mod description_tests {
        use super::*;

        #[test]
        fn table_dropped_description() {
            let table = TableName::try_new("users".to_string()).unwrap();
            let change = BreakingChange::new(BreakingChangeKind::TableDropped { table });

            assert_eq!(change.description, "Table 'users' was dropped");
        }

        #[test]
        fn column_renamed_description() {
            let table = TableName::try_new("users".to_string()).unwrap();
            let from = ColumnName::try_new("email".to_string()).unwrap();
            let to = ColumnName::try_new("email_address".to_string()).unwrap();

            let change = BreakingChange::new(BreakingChangeKind::ColumnRenamed {
                table,
                from,
                to,
                similarity: 0.85,
            });

            assert_eq!(
                change.description,
                "Column 'users.email' was renamed to 'email_address' (similarity: 85%)"
            );
        }
    }
}
