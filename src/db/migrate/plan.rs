//! Migration plan containing ordered operations.
//!
//! A `MigrationPlan` represents a sequence of operations needed to transform
//! a source schema into a target schema. Operations are ordered to respect
//! database object dependencies.

use serde::{Deserialize, Serialize};

use super::collector::{CollectorConfig, OperationCollector};
use super::operation::{Operation, OperationId};
use super::ordering::topological_sort;
use super::render::{RenderedOperation, Renderer};
use super::script::MigrationScript;
use crate::db::diff::NamespaceDiff;

/// Configuration for migration plan generation.
#[derive(Debug, Clone)]
pub struct PlanConfig {
    /// Use CONCURRENTLY for index operations when possible.
    pub concurrent_indexes: bool,
    /// Include comment operations.
    pub include_comments: bool,
}

impl Default for PlanConfig {
    fn default() -> Self {
        Self {
            concurrent_indexes: false,
            include_comments: true,
        }
    }
}

impl PlanConfig {
    /// Create a config that excludes comment operations.
    pub fn without_comments() -> Self {
        Self {
            include_comments: false,
            ..Default::default()
        }
    }

    /// Create a config that uses concurrent index operations.
    pub fn with_concurrent_indexes() -> Self {
        Self {
            concurrent_indexes: true,
            ..Default::default()
        }
    }
}

/// A migration plan containing operations in execution order.
///
/// Operations are ordered to respect dependencies:
/// 1. Drops (in reverse dependency order)
/// 2. Renames
/// 3. Creates for enums and sequences
/// 4. Creates for tables (FK-aware ordering)
/// 5. Creates for constraints and indexes
/// 6. Creates for views
/// 7. Alters
/// 8. Comments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPlan {
    /// Operations in execution order.
    pub operations: Vec<Operation>,
}

impl MigrationPlan {
    /// Create a migration plan from a namespace diff.
    pub fn from_diff(diff: &NamespaceDiff) -> Self {
        Self::from_diff_with_config(diff, &PlanConfig::default())
    }

    /// Create a migration plan with custom configuration.
    pub fn from_diff_with_config(diff: &NamespaceDiff, config: &PlanConfig) -> Self {
        let collector_config = CollectorConfig {
            include_comments: config.include_comments,
            concurrent_indexes: config.concurrent_indexes,
        };
        let collector = OperationCollector::new(&collector_config);
        let operations = collector.collect(diff);
        let ordered = topological_sort(operations);
        Self {
            operations: ordered,
        }
    }

    /// Create an empty migration plan.
    pub fn empty() -> Self {
        Self {
            operations: Vec::new(),
        }
    }

    /// Create a migration plan from a list of operations.
    ///
    /// The operations will be sorted to respect dependencies.
    pub fn from_operations(operations: Vec<Operation>) -> Self {
        let ordered = topological_sort(operations);
        Self {
            operations: ordered,
        }
    }

    /// Returns true if the plan has no operations.
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Returns the number of operations.
    pub fn len(&self) -> usize {
        self.operations.len()
    }

    /// Iterate over operations.
    pub fn iter(&self) -> impl Iterator<Item = &Operation> {
        self.operations.iter()
    }

    /// Iterate over operations with their IDs.
    pub fn iter_with_ids(&self) -> impl Iterator<Item = (OperationId, &Operation)> {
        self.operations
            .iter()
            .enumerate()
            .map(|(i, op)| (OperationId(i), op))
    }

    /// Render the plan to a migration script using the given renderer.
    pub fn render<R: Renderer>(&self, renderer: &R) -> MigrationScript {
        let rendered: Vec<RenderedOperation> = self
            .operations
            .iter()
            .map(|op| renderer.render(op))
            .collect();
        MigrationScript::new(rendered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::diff::Diff;
    use crate::db::model::{EnumType, Table, TableKind};
    use crate::db::schema::{Oid, SchemaName, TableName, TypeName};

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

    fn simple_table(name: &str) -> Table {
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

    fn simple_enum(name: &str) -> EnumType {
        EnumType {
            oid: Oid::new(1),
            name: TypeName::try_new(name.to_string()).unwrap(),
            values: vec!["a".to_string(), "b".to_string()],
            comment: None,
        }
    }

    #[test]
    fn empty_diff_produces_empty_plan() {
        let diff = empty_namespace_diff(test_schema());
        let plan = MigrationPlan::from_diff(&diff);
        assert!(plan.is_empty());
        assert_eq!(plan.len(), 0);
    }

    #[test]
    fn plan_from_diff_with_tables() {
        let mut diff = empty_namespace_diff(test_schema());
        diff.tables.added.push(simple_table("users"));

        let plan = MigrationPlan::from_diff(&diff);
        assert!(!plan.is_empty());
        assert_eq!(plan.len(), 1);
    }

    #[test]
    fn plan_orders_enums_before_tables() {
        let mut diff = empty_namespace_diff(test_schema());
        diff.tables.added.push(simple_table("users"));
        diff.enums.added.push(simple_enum("status"));

        let plan = MigrationPlan::from_diff(&diff);
        assert_eq!(plan.len(), 2);

        // Enum should come first
        assert!(matches!(&plan.operations[0], Operation::CreateEnum { .. }));
        assert!(matches!(&plan.operations[1], Operation::CreateTable { .. }));
    }

    #[test]
    fn iter_with_ids() {
        let mut diff = empty_namespace_diff(test_schema());
        diff.tables.added.push(simple_table("users"));
        diff.tables.added.push(simple_table("posts"));

        let plan = MigrationPlan::from_diff(&diff);

        let ids: Vec<_> = plan.iter_with_ids().map(|(id, _)| id.0).collect();
        assert_eq!(ids, vec![0, 1]);
    }

    #[test]
    fn config_without_comments() {
        let config = PlanConfig::without_comments();
        assert!(!config.include_comments);
    }

    #[test]
    fn from_operations_sorts() {
        let ops = vec![
            Operation::CreateTable {
                schema: test_schema(),
                table: simple_table("users"),
            },
            Operation::CreateEnum {
                schema: test_schema(),
                enum_type: simple_enum("status"),
            },
        ];

        let plan = MigrationPlan::from_operations(ops);

        // Enum should be sorted first
        assert!(matches!(&plan.operations[0], Operation::CreateEnum { .. }));
        assert!(matches!(&plan.operations[1], Operation::CreateTable { .. }));
    }
}
