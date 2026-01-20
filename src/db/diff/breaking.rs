//! Breaking change detection for schema diffs.
//!
//! This module provides functionality for analyzing schema diffs to identify
//! breaking changes that would disrupt running applications. Breaking changes
//! require careful migration strategies to avoid downtime.
//!
//! # Overview
//!
//! A schema change is either **safe** (can be deployed directly) or **breaking**
//! (requires a mitigation strategy). There is no middle ground—if a change might
//! fail or disrupt running applications, it is breaking.
//!
//! Breaking changes are classified by their **mitigation strategy**, which describes
//! how to safely execute the change:
//!
//! | Strategy | Description | Examples |
//! |----------|-------------|----------|
//! | `DualWrite` | Requires parallel structures with synchronized writes | Rename column, rename table, change column type |
//! | `Backfill` | Requires populating data before completion | Add NOT NULL to existing column |
//! | `Ratchet` | Requires NOT VALID + backfill + VALIDATE pattern | Add UNIQUE/CHECK/FK constraint |
//! | `Destructive` | Intentionally removes data/structure (irreversible) | Drop table, drop column, remove enum value |
//!
//! # Example
//!
//! ```
//! use tern::db::diff::{diff_namespaces, NamespaceDiff};
//! use tern::db::diff::breaking::{analyze_breaking_changes, MitigationStrategy};
//! use tern::db::model::Namespace;
//!
//! # fn example(source: Namespace, target: Namespace) {
//! let diff = diff_namespaces(&source, &target);
//! let analysis = analyze_breaking_changes(&diff);
//!
//! if !analysis.is_safe() {
//!     println!("Found {} breaking changes:", analysis.len());
//!     for change in analysis.iter() {
//!         println!("  [{}] {}", change.mitigation.as_str(), change.description);
//!     }
//! }
//! # }
//! ```
//!
//! # Safe Changes
//!
//! These operations are safe and don't require mitigation:
//!
//! - Adding new tables, views, sequences
//! - Adding nullable columns
//! - Making columns nullable (NOT NULL → nullable)
//! - Dropping constraints
//! - Adding/dropping indexes (performance impact only)
//! - Adding enum values
//! - Widening column types (e.g., integer → bigint, varchar(50) → varchar(100))

use serde::{Deserialize, Serialize};

use crate::db::model::Constraint;
use crate::db::model::constraint::ConstraintKind;
use crate::db::model::types::{QualifiedTableName, SqlExpr, TypeInfo};
use crate::db::schema::{
    ColumnName, ConstraintName, SchemaName, SequenceName, TableName, TypeName,
};

use super::schema_diff::{ModifiedColumn, ModifiedTable, NamespaceDiff};

// =============================================================================
// Mitigation Strategy
// =============================================================================

/// Strategy for safely executing a breaking change.
///
/// Each breaking change has an associated mitigation strategy that describes
/// the pattern needed to execute it without downtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MitigationStrategy {
    /// Requires a period where both old and new structures coexist with synchronized writes.
    ///
    /// Pattern:
    /// 1. Add new structure (column, table)
    /// 2. Deploy application that writes to BOTH old and new
    /// 3. Backfill new structure from old
    /// 4. Deploy application that reads from new
    /// 5. Deploy application that writes ONLY to new
    /// 6. Drop old structure
    ///
    /// Examples: rename column, rename table, change column type
    DualWrite,

    /// Requires populating data before the change can complete.
    ///
    /// Pattern:
    /// 1. Add constraint with NOT VALID (creates the ratchet)
    /// 2. Backfill/update any rows that don't satisfy the constraint
    /// 3. Validate the constraint
    /// 4. Optionally add the actual column constraint (e.g., NOT NULL)
    ///
    /// Examples: add NOT NULL to existing column
    Backfill,

    /// Requires the NOT VALID + backfill + VALIDATE pattern.
    ///
    /// Pattern:
    /// 1. Add constraint with NOT VALID (instant, non-blocking)
    /// 2. New inserts/updates are now validated (the "ratchet" is engaged)
    /// 3. Fix any existing rows that violate the constraint
    /// 4. VALIDATE CONSTRAINT to verify all data complies
    ///
    /// Examples: add UNIQUE, CHECK, FK, PK constraints
    Ratchet,

    /// Intentionally removes data or structure. Irreversible.
    ///
    /// Pattern:
    /// 1. Verify nothing references the object (application code, other objects)
    /// 2. Wait for all old application instances to drain
    /// 3. Perform the drop
    ///
    /// Note: This may be intentional cleanup, but represents data loss risk.
    ///
    /// Examples: drop table, drop column, remove enum value
    Destructive,
}

impl MitigationStrategy {
    /// Returns a short string representation of the strategy.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::DualWrite => "dual-write",
            Self::Backfill => "backfill",
            Self::Ratchet => "ratchet",
            Self::Destructive => "destructive",
        }
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
    /// Returns the mitigation strategy for this kind of change.
    pub fn mitigation(&self) -> MitigationStrategy {
        match self {
            // Destructive: drops and removals
            Self::TableDropped { .. }
            | Self::ColumnDropped { .. }
            | Self::ViewDropped { .. }
            | Self::SequenceDropped { .. }
            | Self::EnumValueRemoved { .. }
            | Self::EnumValuesReordered { .. } => MitigationStrategy::Destructive,

            // DualWrite: renames and type changes
            Self::TableRenamed { .. }
            | Self::ColumnRenamed { .. }
            | Self::ColumnTypeChanged { .. }
            | Self::ViewRenamed { .. }
            | Self::SequenceRenamed { .. }
            | Self::MaterializationChanged { .. } => MitigationStrategy::DualWrite,

            // Backfill: nullability changes
            Self::ColumnMadeNonNullable { .. } => MitigationStrategy::Backfill,

            // Ratchet: constraint additions
            Self::PrimaryKeyAdded { .. }
            | Self::UniqueConstraintAdded { .. }
            | Self::CheckConstraintAdded { .. }
            | Self::ForeignKeyAdded { .. }
            | Self::ExclusionConstraintAdded { .. } => MitigationStrategy::Ratchet,
        }
    }
}

// =============================================================================
// Breaking Change
// =============================================================================

/// A detected breaking change with full context.
///
/// Contains the specific change kind, its mitigation strategy, and a human-readable description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakingChange {
    /// The specific kind of breaking change.
    pub kind: BreakingChangeKind,
    /// The strategy for safely executing this change.
    pub mitigation: MitigationStrategy,
    /// Human-readable description of the change.
    pub description: String,
}

impl BreakingChange {
    /// Creates a new breaking change from a kind.
    pub fn new(kind: BreakingChangeKind) -> Self {
        let mitigation = kind.mitigation();
        let description = Self::describe(&kind);
        Self {
            kind,
            mitigation,
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
/// Contains all detected breaking changes. If empty, the diff is safe to apply directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakingChangeAnalysis {
    /// The schema that was analyzed.
    pub schema: SchemaName,
    /// All detected breaking changes.
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

    /// Returns true if there are no breaking changes (safe to apply directly).
    pub fn is_safe(&self) -> bool {
        self.changes.is_empty()
    }

    /// Returns the number of breaking changes.
    pub fn len(&self) -> usize {
        self.changes.len()
    }

    /// Returns true if there are no breaking changes.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Returns an iterator over all breaking changes.
    pub fn iter(&self) -> impl Iterator<Item = &BreakingChange> {
        self.changes.iter()
    }

    /// Returns an iterator over changes with a specific mitigation strategy.
    pub fn by_mitigation(
        &self,
        strategy: MitigationStrategy,
    ) -> impl Iterator<Item = &BreakingChange> {
        self.changes
            .iter()
            .filter(move |c| c.mitigation == strategy)
    }

    /// Returns the count of changes requiring a specific mitigation strategy.
    pub fn count_by_mitigation(&self, strategy: MitigationStrategy) -> usize {
        self.changes
            .iter()
            .filter(|c| c.mitigation == strategy)
            .count()
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
/// Examines all changes in the diff and identifies those that require
/// mitigation strategies. Safe changes (like adding nullable columns)
/// are not included in the result.
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
/// if analysis.is_safe() {
///     println!("Migration is safe to apply directly");
/// } else {
///     println!("Found {} breaking changes requiring mitigation", analysis.len());
/// }
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
    // Dropped tables are breaking (Destructive)
    for table_name in &diff.tables.removed {
        analysis.add(BreakingChange::new(BreakingChangeKind::TableDropped {
            table: table_name.clone(),
        }));
    }

    // Renamed tables are breaking (DualWrite)
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

    // Dropped columns are breaking (Destructive)
    for column_name in &table.columns.removed {
        analysis.add(BreakingChange::new(BreakingChangeKind::ColumnDropped {
            table: table_name.clone(),
            column: column_name.clone(),
        }));
    }

    // Renamed columns are breaking (DualWrite)
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

    // Analyze added constraints (all are breaking with Ratchet mitigation)
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
    // Type changes are potentially breaking (DualWrite)
    if let Some(type_change) = &column.type_info
        && is_breaking_type_change(&type_change.source, &type_change.target)
    {
        analysis.add(BreakingChange::new(BreakingChangeKind::ColumnTypeChanged {
            table: table_name.clone(),
            column: column.name.clone(),
            from_type: type_change.source.clone(),
            to_type: type_change.target.clone(),
        }));
    }

    // Nullable to non-nullable is breaking (Backfill)
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

/// Analyzes an added constraint for breaking changes.
///
/// All constraint additions are breaking because they may fail on existing data.
/// They require the Ratchet mitigation strategy.
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
    // Dropped views are breaking (Destructive)
    for view_name in &diff.views.removed {
        analysis.add(BreakingChange::new(BreakingChangeKind::ViewDropped {
            view: view_name.clone(),
        }));
    }

    // Renamed views are breaking (DualWrite)
    for rename in &diff.views.potential_renames {
        analysis.add(BreakingChange::new(BreakingChangeKind::ViewRenamed {
            from: rename.source_key.clone(),
            to: rename.target.name.clone(),
            similarity: rename.similarity,
        }));
    }

    // Check for materialization changes (DualWrite)
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
    // Dropped sequences are breaking (Destructive)
    for seq_name in &diff.sequences.removed {
        analysis.add(BreakingChange::new(BreakingChangeKind::SequenceDropped {
            sequence: seq_name.clone(),
        }));
    }

    // Renamed sequences are breaking (DualWrite)
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
        // Removed values are breaking (Destructive)
        if !modified_enum.values_removed.is_empty() {
            analysis.add(BreakingChange::new(BreakingChangeKind::EnumValueRemoved {
                enum_type: modified_enum.name.clone(),
                values: modified_enum.values_removed.clone(),
            }));
        }

        // Reordered values are breaking (Destructive - requires enum recreation)
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

/// Determines if a column type change is breaking.
///
/// Returns `true` if the change requires mitigation, `false` if it's safe.
///
/// Safe changes (returns false):
/// - Same type
/// - Widening: smallint → integer → bigint
/// - Widening: real → double precision
/// - Widening: varchar(n) → varchar(m) where m > n
/// - Widening: varchar → text
///
/// Breaking changes (returns true):
/// - Narrowing: bigint → integer → smallint
/// - Narrowing: double precision → real
/// - Narrowing: varchar(m) → varchar(n) where n < m
/// - Narrowing: text → varchar
/// - Any other type change
fn is_breaking_type_change(from: &TypeInfo, to: &TypeInfo) -> bool {
    // Same type with same formatting - not a real change
    if from.formatted == to.formatted {
        return false;
    }

    // Check for known safe widening operations
    if is_safe_type_widening(from, to) {
        return false;
    }

    // All other type changes are breaking
    true
}

/// Checks if a type change is a safe widening operation.
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

    mod mitigation_strategy_tests {
        use super::*;

        #[test]
        fn as_str_returns_correct_strings() {
            assert_eq!(MitigationStrategy::DualWrite.as_str(), "dual-write");
            assert_eq!(MitigationStrategy::Backfill.as_str(), "backfill");
            assert_eq!(MitigationStrategy::Ratchet.as_str(), "ratchet");
            assert_eq!(MitigationStrategy::Destructive.as_str(), "destructive");
        }
    }

    mod kind_mitigation_tests {
        use super::*;

        #[test]
        fn drops_are_destructive() {
            let table = TableName::try_new("t".to_string()).unwrap();
            let column = ColumnName::try_new("c".to_string()).unwrap();
            let view = TableName::try_new("v".to_string()).unwrap();
            let seq = SequenceName::try_new("s".to_string()).unwrap();
            let enum_type = TypeName::try_new("e".to_string()).unwrap();

            assert_eq!(
                BreakingChangeKind::TableDropped { table }.mitigation(),
                MitigationStrategy::Destructive
            );
            assert_eq!(
                BreakingChangeKind::ColumnDropped {
                    table: TableName::try_new("t".to_string()).unwrap(),
                    column
                }
                .mitigation(),
                MitigationStrategy::Destructive
            );
            assert_eq!(
                BreakingChangeKind::ViewDropped { view }.mitigation(),
                MitigationStrategy::Destructive
            );
            assert_eq!(
                BreakingChangeKind::SequenceDropped { sequence: seq }.mitigation(),
                MitigationStrategy::Destructive
            );
            assert_eq!(
                BreakingChangeKind::EnumValueRemoved {
                    enum_type: enum_type.clone(),
                    values: vec![]
                }
                .mitigation(),
                MitigationStrategy::Destructive
            );
            assert_eq!(
                BreakingChangeKind::EnumValuesReordered { enum_type }.mitigation(),
                MitigationStrategy::Destructive
            );
        }

        #[test]
        fn renames_are_dual_write() {
            let from = TableName::try_new("old".to_string()).unwrap();
            let to = TableName::try_new("new".to_string()).unwrap();

            assert_eq!(
                BreakingChangeKind::TableRenamed {
                    from,
                    to,
                    similarity: 0.9
                }
                .mitigation(),
                MitigationStrategy::DualWrite
            );
        }

        #[test]
        fn constraints_are_ratchet() {
            let table = TableName::try_new("t".to_string()).unwrap();
            let constraint = ConstraintName::try_new("c".to_string()).unwrap();
            let column = ColumnName::try_new("col".to_string()).unwrap();

            assert_eq!(
                BreakingChangeKind::UniqueConstraintAdded {
                    table: table.clone(),
                    constraint: constraint.clone(),
                    columns: vec![column.clone()]
                }
                .mitigation(),
                MitigationStrategy::Ratchet
            );
            assert_eq!(
                BreakingChangeKind::PrimaryKeyAdded {
                    table: table.clone(),
                    constraint: constraint.clone(),
                    columns: vec![column]
                }
                .mitigation(),
                MitigationStrategy::Ratchet
            );
            assert_eq!(
                BreakingChangeKind::CheckConstraintAdded {
                    table,
                    constraint,
                    expression: SqlExpr::new("x > 0".to_string())
                }
                .mitigation(),
                MitigationStrategy::Ratchet
            );
        }

        #[test]
        fn non_nullable_is_backfill() {
            let table = TableName::try_new("t".to_string()).unwrap();
            let column = ColumnName::try_new("c".to_string()).unwrap();

            assert_eq!(
                BreakingChangeKind::ColumnMadeNonNullable { table, column }.mitigation(),
                MitigationStrategy::Backfill
            );
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
        fn same_type_is_safe() {
            let t = make_type_info("int4", "integer");
            assert!(!is_breaking_type_change(&t, &t));
        }

        #[test]
        fn integer_widening_is_safe() {
            let small = make_type_info("int2", "smallint");
            let int = make_type_info("int4", "integer");
            let big = make_type_info("int8", "bigint");

            assert!(!is_breaking_type_change(&small, &int));
            assert!(!is_breaking_type_change(&small, &big));
            assert!(!is_breaking_type_change(&int, &big));
        }

        #[test]
        fn integer_narrowing_is_breaking() {
            let small = make_type_info("int2", "smallint");
            let int = make_type_info("int4", "integer");
            let big = make_type_info("int8", "bigint");

            assert!(is_breaking_type_change(&big, &int));
            assert!(is_breaking_type_change(&big, &small));
            assert!(is_breaking_type_change(&int, &small));
        }

        #[test]
        fn varchar_widening_is_safe() {
            let v100 = make_type_info("varchar", "character varying(100)");
            let v200 = make_type_info("varchar", "character varying(200)");
            let vunlimited = make_type_info("varchar", "character varying");

            assert!(!is_breaking_type_change(&v100, &v200));
            assert!(!is_breaking_type_change(&v100, &vunlimited));
        }

        #[test]
        fn varchar_narrowing_is_breaking() {
            let v100 = make_type_info("varchar", "character varying(100)");
            let v200 = make_type_info("varchar", "character varying(200)");
            let vunlimited = make_type_info("varchar", "character varying");

            assert!(is_breaking_type_change(&v200, &v100));
            assert!(is_breaking_type_change(&vunlimited, &v100));
        }

        #[test]
        fn varchar_to_text_is_safe() {
            let v = make_type_info("varchar", "character varying(100)");
            let t = make_type_info("text", "text");

            assert!(!is_breaking_type_change(&v, &t));
        }

        #[test]
        fn text_to_varchar_is_breaking() {
            let v = make_type_info("varchar", "character varying(100)");
            let t = make_type_info("text", "text");

            assert!(is_breaking_type_change(&t, &v));
        }

        #[test]
        fn float_widening_is_safe() {
            let real = make_type_info("float4", "real");
            let double = make_type_info("float8", "double precision");

            assert!(!is_breaking_type_change(&real, &double));
        }

        #[test]
        fn float_narrowing_is_breaking() {
            let real = make_type_info("float4", "real");
            let double = make_type_info("float8", "double precision");

            assert!(is_breaking_type_change(&double, &real));
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
        fn empty_analysis_is_safe() {
            let schema = SchemaName::try_new("public".to_string()).unwrap();
            let analysis = BreakingChangeAnalysis::new(schema);

            assert!(analysis.is_safe());
            assert!(analysis.is_empty());
            assert_eq!(analysis.len(), 0);
        }

        #[test]
        fn analysis_with_change_is_not_safe() {
            let schema = SchemaName::try_new("public".to_string()).unwrap();
            let mut analysis = BreakingChangeAnalysis::new(schema);

            let table = TableName::try_new("users".to_string()).unwrap();
            analysis.add(BreakingChange::new(BreakingChangeKind::TableDropped {
                table,
            }));

            assert!(!analysis.is_safe());
            assert!(!analysis.is_empty());
            assert_eq!(analysis.len(), 1);
        }

        #[test]
        fn count_by_mitigation() {
            let schema = SchemaName::try_new("public".to_string()).unwrap();
            let mut analysis = BreakingChangeAnalysis::new(schema);

            // Add one destructive
            analysis.add(BreakingChange::new(BreakingChangeKind::TableDropped {
                table: TableName::try_new("t1".to_string()).unwrap(),
            }));

            // Add two ratchet
            analysis.add(BreakingChange::new(
                BreakingChangeKind::UniqueConstraintAdded {
                    table: TableName::try_new("t".to_string()).unwrap(),
                    constraint: ConstraintName::try_new("c1".to_string()).unwrap(),
                    columns: vec![],
                },
            ));
            analysis.add(BreakingChange::new(
                BreakingChangeKind::UniqueConstraintAdded {
                    table: TableName::try_new("t".to_string()).unwrap(),
                    constraint: ConstraintName::try_new("c2".to_string()).unwrap(),
                    columns: vec![],
                },
            ));

            assert_eq!(
                analysis.count_by_mitigation(MitigationStrategy::Destructive),
                1
            );
            assert_eq!(analysis.count_by_mitigation(MitigationStrategy::Ratchet), 2);
            assert_eq!(
                analysis.count_by_mitigation(MitigationStrategy::DualWrite),
                0
            );
        }
    }

    mod description_tests {
        use super::*;

        #[test]
        fn table_dropped_description() {
            let table = TableName::try_new("users".to_string()).unwrap();
            let change = BreakingChange::new(BreakingChangeKind::TableDropped { table });

            assert_eq!(change.description, "Table 'users' was dropped");
            assert_eq!(change.mitigation, MitigationStrategy::Destructive);
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
            assert_eq!(change.mitigation, MitigationStrategy::DualWrite);
        }
    }
}
