//! Extracts operations from namespace diffs.
//!
//! The collector walks a `NamespaceDiff` and produces a sequence of `Operation`s
//! that can transform the source schema into the target schema.

use crate::db::diff::{
    EnumDiff, ModifiedColumn, ModifiedEnum, ModifiedSequence, ModifiedTable, ModifiedView,
    NamespaceDiff, SequenceDiff, TableDiff, ViewDiff,
};
use crate::db::migrate::operation::{
    ColumnChanges, CommentTarget, DefaultChange, EnumValuePosition, GeneratedChange,
    IdentityChange, Operation, SequenceChanges, SetColumnType,
};
use crate::db::model::constraint::ConstraintKind;
use crate::db::schema::SchemaName;

/// Configuration for operation collection.
#[derive(Debug, Clone)]
pub struct CollectorConfig {
    /// Whether to generate operations for comment changes.
    pub include_comments: bool,
    /// Whether to use CONCURRENTLY for index operations.
    pub concurrent_indexes: bool,
}

impl Default for CollectorConfig {
    fn default() -> Self {
        Self {
            include_comments: true,
            concurrent_indexes: false,
        }
    }
}

/// Collects operations from a namespace diff.
pub struct OperationCollector<'a> {
    config: &'a CollectorConfig,
    schema: Option<SchemaName>,
    operations: Vec<Operation>,
}

impl<'a> OperationCollector<'a> {
    /// Create a new collector with the given configuration.
    pub fn new(config: &'a CollectorConfig) -> Self {
        Self {
            config,
            schema: None,
            operations: Vec::new(),
        }
    }

    /// Collect all operations from a namespace diff.
    ///
    /// The operations are returned in an order that respects dependencies:
    /// 1. Drops (views, indexes, constraints, columns, tables, sequences, enums)
    /// 2. Creates for independent objects (enums, sequences)
    /// 3. Table creates and modifications
    /// 4. View creates and modifications
    /// 5. Comment updates (if enabled)
    pub fn collect(mut self, diff: &NamespaceDiff) -> Vec<Operation> {
        self.schema = Some(diff.name.clone());

        // Order matters for dependency correctness:

        // 1. Drop dependent objects first (reverse dependency order)
        self.collect_drops(diff);

        // 2. Handle renames (must happen after drops, before creates)
        self.collect_renames(diff);

        // 3. Create/modify independent objects
        self.collect_enums(&diff.enums);
        self.collect_sequences(&diff.sequences);

        // 4. Create/modify tables
        self.collect_tables(&diff.tables);

        // 5. Create/modify views (depend on tables)
        self.collect_views(&diff.views);

        // 6. Comments last
        if self.config.include_comments {
            self.collect_comments(diff);
        }

        self.operations
    }

    fn schema(&self) -> &SchemaName {
        self.schema.as_ref().expect("schema not set")
    }

    // =========================================================================
    // Drop Collection (reverse dependency order)
    // =========================================================================

    fn collect_drops(&mut self, diff: &NamespaceDiff) {
        // Drop views first (depend on tables)
        for name in &diff.views.removed {
            // Note: We don't have full View info to know if it was materialized
            // The diff only gives us the name for removed items
            self.operations.push(Operation::DropView {
                schema: self.schema().clone(),
                name: name.clone(),
                is_materialized: false,
            });
        }

        // Drop table indexes, constraints, columns for modified tables
        for modified in &diff.tables.modified {
            self.collect_table_drops(modified);
        }

        // Drop tables
        for name in &diff.tables.removed {
            self.operations.push(Operation::DropTable {
                schema: self.schema().clone(),
                name: name.clone(),
            });
        }

        // Drop sequences (may be used by tables)
        for name in &diff.sequences.removed {
            self.operations.push(Operation::DropSequence {
                schema: self.schema().clone(),
                name: name.clone(),
            });
        }

        // Drop enums last (tables may depend on them)
        for name in &diff.enums.removed {
            self.operations.push(Operation::DropEnum {
                schema: self.schema().clone(),
                name: name.clone(),
            });
        }
    }

    fn collect_table_drops(&mut self, modified: &ModifiedTable) {
        // Drop indexes first
        for name in &modified.indexes.removed {
            self.operations.push(Operation::DropIndex {
                schema: self.schema().clone(),
                name: name.clone(),
                concurrently: self.config.concurrent_indexes,
            });
        }

        // Drop constraints
        for name in &modified.constraints.removed {
            self.operations.push(Operation::DropConstraint {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                name: name.clone(),
            });
        }

        // Handle modified constraints (drop + recreate)
        for mod_constraint in &modified.constraints.modified {
            self.operations.push(Operation::DropConstraint {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                name: mod_constraint.name.clone(),
            });
        }

        // Handle modified indexes (drop + recreate)
        for mod_index in &modified.indexes.modified {
            self.operations.push(Operation::DropIndex {
                schema: self.schema().clone(),
                name: mod_index.name.clone(),
                concurrently: self.config.concurrent_indexes,
            });
        }

        // Drop columns
        for name in &modified.columns.removed {
            self.operations.push(Operation::DropColumn {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                name: name.clone(),
            });
        }
    }

    // =========================================================================
    // Rename Collection
    // =========================================================================

    fn collect_renames(&mut self, diff: &NamespaceDiff) {
        // Rename tables
        for rename in &diff.tables.potential_renames {
            self.operations.push(Operation::RenameTable {
                schema: self.schema().clone(),
                from: rename.source_key.clone(),
                to: rename.target.name.clone(),
            });
        }

        // Rename enums
        for rename in &diff.enums.potential_renames {
            self.operations.push(Operation::RenameEnum {
                schema: self.schema().clone(),
                from: rename.source_key.clone(),
                to: rename.target.name.clone(),
            });
        }

        // Rename sequences
        for rename in &diff.sequences.potential_renames {
            self.operations.push(Operation::RenameSequence {
                schema: self.schema().clone(),
                from: rename.source_key.clone(),
                to: rename.target.name.clone(),
            });
        }

        // Rename views
        for rename in &diff.views.potential_renames {
            self.operations.push(Operation::RenameView {
                schema: self.schema().clone(),
                from: rename.source_key.clone(),
                to: rename.target.name.clone(),
                is_materialized: rename.target.is_materialized,
            });
        }
    }

    // =========================================================================
    // Enum Collection
    // =========================================================================

    fn collect_enums(&mut self, diff: &EnumDiff) {
        // Create new enums
        for enum_type in &diff.added {
            self.operations.push(Operation::CreateEnum {
                schema: self.schema().clone(),
                enum_type: enum_type.clone(),
            });
        }

        // Modify existing enums (add values)
        for modified in &diff.modified {
            self.collect_enum_modifications(modified);
        }
    }

    fn collect_enum_modifications(&mut self, modified: &ModifiedEnum) {
        // PostgreSQL only allows adding enum values, not removing or reordering
        // New values must be added one at a time
        for value in &modified.values_added {
            self.operations.push(Operation::AddEnumValue {
                schema: self.schema().clone(),
                enum_name: modified.name.clone(),
                value: value.clone(),
                position: EnumValuePosition::End,
            });
        }
    }

    // =========================================================================
    // Sequence Collection
    // =========================================================================

    fn collect_sequences(&mut self, diff: &SequenceDiff) {
        // Create new sequences
        for sequence in &diff.added {
            self.operations.push(Operation::CreateSequence {
                schema: self.schema().clone(),
                sequence: sequence.clone(),
            });
        }

        // Modify existing sequences
        for modified in &diff.modified {
            if let Some(changes) = self.build_sequence_changes(modified) {
                self.operations.push(Operation::AlterSequence {
                    schema: self.schema().clone(),
                    name: modified.name.clone(),
                    changes,
                });
            }
        }
    }

    fn build_sequence_changes(&self, modified: &ModifiedSequence) -> Option<SequenceChanges> {
        let changes = SequenceChanges {
            data_type: modified.data_type.as_ref().map(|c| c.target.clone()),
            increment: modified.increment.as_ref().map(|c| c.target),
            min_value: modified.min_value.as_ref().map(|c| c.target),
            max_value: modified.max_value.as_ref().map(|c| c.target),
            start_value: modified.start_value.as_ref().map(|c| c.target),
            cache_size: modified.cache_size.as_ref().map(|c| c.target),
            is_cyclic: modified.is_cyclic.as_ref().map(|c| c.target),
        };

        if changes.is_empty() {
            None
        } else {
            Some(changes)
        }
    }

    // =========================================================================
    // Table Collection
    // =========================================================================

    fn collect_tables(&mut self, diff: &TableDiff) {
        // Create new tables
        for table in &diff.added {
            self.operations.push(Operation::CreateTable {
                schema: self.schema().clone(),
                table: table.clone(),
            });

            // Add non-inline constraints (FK, exclusion) separately
            for constraint in &table.constraints {
                if self.is_separate_constraint(&constraint.kind) {
                    self.operations.push(Operation::AddConstraint {
                        schema: self.schema().clone(),
                        table: table.name.clone(),
                        constraint: constraint.clone(),
                    });
                }
            }

            // Add indexes (non-constraint-backing)
            for index in &table.indexes {
                if !index.is_constraint_index {
                    self.operations.push(Operation::CreateIndex {
                        schema: self.schema().clone(),
                        table: table.name.clone(),
                        index: index.clone(),
                        concurrently: self.config.concurrent_indexes,
                    });
                }
            }
        }

        // Modify existing tables
        for modified in &diff.modified {
            self.collect_table_modifications(modified);
        }
    }

    fn collect_table_modifications(&mut self, modified: &ModifiedTable) {
        // Handle column renames first
        for rename in &modified.columns.potential_renames {
            self.operations.push(Operation::RenameColumn {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                from: rename.source_key.clone(),
                to: rename.target.name.clone(),
            });
        }

        // Handle constraint renames
        for rename in &modified.constraints.potential_renames {
            self.operations.push(Operation::RenameConstraint {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                from: rename.source_key.clone(),
                to: rename.target.name.clone(),
            });
        }

        // Handle index renames
        for rename in &modified.indexes.potential_renames {
            self.operations.push(Operation::RenameIndex {
                schema: self.schema().clone(),
                from: rename.source_key.clone(),
                to: rename.target.name.clone(),
            });
        }

        // Add new columns
        for column in &modified.columns.added {
            self.operations.push(Operation::AddColumn {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                column: column.clone(),
            });
        }

        // Modify existing columns
        for mod_col in &modified.columns.modified {
            if let Some(changes) = self.build_column_changes(mod_col) {
                self.operations.push(Operation::AlterColumn {
                    schema: self.schema().clone(),
                    table: modified.name.clone(),
                    name: mod_col.name.clone(),
                    changes,
                });
            }
        }

        // Add new constraints
        for constraint in &modified.constraints.added {
            self.operations.push(Operation::AddConstraint {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                constraint: constraint.clone(),
            });
        }

        // Recreate modified constraints (already dropped above)
        for mod_constraint in &modified.constraints.modified {
            self.operations.push(Operation::AddConstraint {
                schema: self.schema().clone(),
                table: modified.name.clone(),
                constraint: mod_constraint.target.clone(),
            });
        }

        // Add new indexes
        for index in &modified.indexes.added {
            if !index.is_constraint_index {
                self.operations.push(Operation::CreateIndex {
                    schema: self.schema().clone(),
                    table: modified.name.clone(),
                    index: index.clone(),
                    concurrently: self.config.concurrent_indexes,
                });
            }
        }

        // Recreate modified indexes (already dropped above)
        for mod_index in &modified.indexes.modified {
            if !mod_index.target.is_constraint_index {
                self.operations.push(Operation::CreateIndex {
                    schema: self.schema().clone(),
                    table: modified.name.clone(),
                    index: mod_index.target.clone(),
                    concurrently: self.config.concurrent_indexes,
                });
            }
        }
    }

    fn build_column_changes(&self, modified: &ModifiedColumn) -> Option<ColumnChanges> {
        let mut changes = ColumnChanges::default();

        if let Some(ref type_change) = modified.type_info {
            changes.set_type = Some(SetColumnType {
                type_info: type_change.target.clone(),
                using: None, // Could be enhanced to suggest USING clause
            });
        }

        if let Some(ref null_change) = modified.is_nullable {
            // set_not_null is the opposite of is_nullable
            changes.set_not_null = Some(!null_change.target);
        }

        if let Some(ref default_change) = modified.default {
            changes.set_default = Some(match &default_change.target {
                Some(expr) => DefaultChange::Set(expr.clone()),
                None => DefaultChange::Drop,
            });
        }

        if let Some(ref identity_change) = modified.identity {
            changes.set_identity = Some(match &identity_change.target {
                Some(kind) => IdentityChange::Add(kind.clone()),
                None => IdentityChange::Drop,
            });
        }

        if let Some(ref generated_change) = modified.generated {
            changes.set_generated = Some(match &generated_change.target {
                Some(generated) => GeneratedChange::Set(generated.clone()),
                None => GeneratedChange::Drop,
            });
        }

        if changes.is_empty() {
            None
        } else {
            Some(changes)
        }
    }

    /// Returns true if this constraint type should be added via ALTER TABLE
    /// rather than inline in CREATE TABLE.
    fn is_separate_constraint(&self, kind: &ConstraintKind) -> bool {
        matches!(
            kind,
            ConstraintKind::ForeignKey(_) | ConstraintKind::Exclusion(_)
        )
    }

    // =========================================================================
    // View Collection
    // =========================================================================

    fn collect_views(&mut self, diff: &ViewDiff) {
        // Create new views
        for view in &diff.added {
            self.operations.push(Operation::CreateView {
                schema: self.schema().clone(),
                view: view.clone(),
            });
        }

        // Modify existing views
        for modified in &diff.modified {
            self.collect_view_modifications(modified);
        }
    }

    fn collect_view_modifications(&mut self, modified: &ModifiedView) {
        // Check if materialized status changed
        if modified.is_materialized.is_some() {
            // Cannot change materialized status in place - would need drop + recreate
            // This is a limitation; for now we skip this case
            // A full implementation would need the full view definition
            return;
        }

        // If definition changed, use CREATE OR REPLACE (for non-materialized views)
        if modified.definition.is_some() {
            // We'd need the full View to do this properly
            // For now, this is a placeholder
        }
    }

    // =========================================================================
    // Comment Collection
    // =========================================================================

    fn collect_comments(&mut self, diff: &NamespaceDiff) {
        // Schema comment
        if let Some(ref comment_change) = diff.comment {
            self.operations.push(Operation::SetComment {
                target: CommentTarget::Schema(self.schema().clone()),
                comment: comment_change
                    .target
                    .as_ref()
                    .map(|c| c.as_ref().to_string()),
            });
        }

        // Table and column comments
        for modified in &diff.tables.modified {
            self.collect_table_comments(modified);
        }

        // Enum comments
        for modified in &diff.enums.modified {
            if let Some(ref comment_change) = modified.comment {
                self.operations.push(Operation::SetComment {
                    target: CommentTarget::Type {
                        schema: self.schema().clone(),
                        type_name: modified.name.clone(),
                    },
                    comment: comment_change
                        .target
                        .as_ref()
                        .map(|c| c.as_ref().to_string()),
                });
            }
        }

        // Sequence comments
        for modified in &diff.sequences.modified {
            if let Some(ref comment_change) = modified.comment {
                self.operations.push(Operation::SetComment {
                    target: CommentTarget::Sequence {
                        schema: self.schema().clone(),
                        sequence: modified.name.clone(),
                    },
                    comment: comment_change
                        .target
                        .as_ref()
                        .map(|c| c.as_ref().to_string()),
                });
            }
        }

        // View comments
        for modified in &diff.views.modified {
            if let Some(ref comment_change) = modified.comment {
                self.operations.push(Operation::SetComment {
                    target: CommentTarget::View {
                        schema: self.schema().clone(),
                        view: modified.name.clone(),
                    },
                    comment: comment_change
                        .target
                        .as_ref()
                        .map(|c| c.as_ref().to_string()),
                });
            }
        }
    }

    fn collect_table_comments(&mut self, modified: &ModifiedTable) {
        // Table comment
        if let Some(ref comment_change) = modified.comment {
            self.operations.push(Operation::SetComment {
                target: CommentTarget::Table {
                    schema: self.schema().clone(),
                    table: modified.name.clone(),
                },
                comment: comment_change
                    .target
                    .as_ref()
                    .map(|c| c.as_ref().to_string()),
            });
        }

        // Column comments
        for mod_col in &modified.columns.modified {
            if let Some(ref comment_change) = mod_col.comment {
                self.operations.push(Operation::SetComment {
                    target: CommentTarget::Column {
                        schema: self.schema().clone(),
                        table: modified.name.clone(),
                        column: mod_col.name.clone(),
                    },
                    comment: comment_change
                        .target
                        .as_ref()
                        .map(|c| c.as_ref().to_string()),
                });
            }
        }

        // Constraint comments
        for mod_constraint in &modified.constraints.modified {
            if let Some(ref comment_change) = mod_constraint.comment {
                self.operations.push(Operation::SetComment {
                    target: CommentTarget::Constraint {
                        schema: self.schema().clone(),
                        table: modified.name.clone(),
                        constraint: mod_constraint.name.clone(),
                    },
                    comment: comment_change
                        .target
                        .as_ref()
                        .map(|c| c.as_ref().to_string()),
                });
            }
        }

        // Index comments
        for mod_index in &modified.indexes.modified {
            if let Some(ref comment_change) = mod_index.comment {
                self.operations.push(Operation::SetComment {
                    target: CommentTarget::Index {
                        schema: self.schema().clone(),
                        index: mod_index.name.clone(),
                    },
                    comment: comment_change
                        .target
                        .as_ref()
                        .map(|c| c.as_ref().to_string()),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::diff::Diff;
    use crate::db::model::column::Column;
    use crate::db::model::types::TypeInfo;
    use crate::db::model::{EnumType, Sequence, Table, TableKind, View};
    use crate::db::schema::{ColumnName, Oid, SequenceName, TableName, TypeName};

    fn test_schema() -> SchemaName {
        SchemaName::try_new("public".to_string()).unwrap()
    }

    fn empty_namespace_diff(name: SchemaName) -> NamespaceDiff {
        NamespaceDiff {
            name,
            tables: Diff::new(),
            views: Diff::new(),
            sequences: Diff::new(),
            enums: Diff::new(),
            comment: None,
        }
    }

    fn test_table(name: &str) -> Table {
        Table {
            oid: Oid::new(1),
            name: TableName::try_new(name.to_string()).unwrap(),
            kind: TableKind::Regular,
            columns: vec![],
            constraints: vec![],
            indexes: vec![],
            comment: None,
        }
    }

    fn test_enum(name: &str, values: Vec<&str>) -> EnumType {
        EnumType {
            oid: Oid::new(1),
            name: TypeName::try_new(name.to_string()).unwrap(),
            values: values.into_iter().map(|s| s.to_string()).collect(),
            comment: None,
        }
    }

    #[test]
    fn collect_empty_diff() {
        let config = CollectorConfig::default();
        let collector = OperationCollector::new(&config);
        let diff = empty_namespace_diff(test_schema());

        let operations = collector.collect(&diff);
        assert!(operations.is_empty());
    }

    #[test]
    fn collect_added_table() {
        let config = CollectorConfig::default();
        let collector = OperationCollector::new(&config);

        let mut diff = empty_namespace_diff(test_schema());
        diff.tables.added.push(test_table("users"));

        let operations = collector.collect(&diff);
        assert_eq!(operations.len(), 1);
        assert!(matches!(&operations[0], Operation::CreateTable { .. }));
    }

    #[test]
    fn collect_removed_table() {
        let config = CollectorConfig::default();
        let collector = OperationCollector::new(&config);

        let mut diff = empty_namespace_diff(test_schema());
        diff.tables
            .removed
            .push(TableName::try_new("old_table".to_string()).unwrap());

        let operations = collector.collect(&diff);
        assert_eq!(operations.len(), 1);
        assert!(matches!(&operations[0], Operation::DropTable { .. }));
    }

    #[test]
    fn collect_added_enum() {
        let config = CollectorConfig::default();
        let collector = OperationCollector::new(&config);

        let mut diff = empty_namespace_diff(test_schema());
        diff.enums
            .added
            .push(test_enum("status", vec!["active", "inactive"]));

        let operations = collector.collect(&diff);
        assert_eq!(operations.len(), 1);
        assert!(matches!(&operations[0], Operation::CreateEnum { .. }));
    }

    #[test]
    fn collect_drops_before_creates() {
        let config = CollectorConfig::default();
        let collector = OperationCollector::new(&config);

        let mut diff = empty_namespace_diff(test_schema());
        diff.tables.added.push(test_table("new_table"));
        diff.tables
            .removed
            .push(TableName::try_new("old_table".to_string()).unwrap());

        let operations = collector.collect(&diff);
        assert_eq!(operations.len(), 2);

        // Drop should come before create
        assert!(matches!(&operations[0], Operation::DropTable { .. }));
        assert!(matches!(&operations[1], Operation::CreateTable { .. }));
    }

    #[test]
    fn collect_enum_modifications() {
        let config = CollectorConfig::default();
        let collector = OperationCollector::new(&config);

        let mut diff = empty_namespace_diff(test_schema());
        diff.enums.modified.push(ModifiedEnum {
            name: TypeName::try_new("status".to_string()).unwrap(),
            values_added: vec!["pending".to_string()],
            values_removed: vec![],
            values_reordered: false,
            comment: None,
        });

        let operations = collector.collect(&diff);
        assert_eq!(operations.len(), 1);
        assert!(matches!(
            &operations[0],
            Operation::AddEnumValue { value, .. } if value == "pending"
        ));
    }
}
