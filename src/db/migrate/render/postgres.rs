//! PostgreSQL-specific SQL rendering.

use crate::db::migrate::operation::{
    ColumnChanges, CommentTarget, DefaultChange, EnumValuePosition, GeneratedChange,
    IdentityChange, Operation, SequenceChanges, SetColumnType,
};
use crate::db::model::column::Column;
use crate::db::model::constraint::{Constraint, ConstraintKind};
use crate::db::model::index::{Index, IndexColumn, NullsOrder, SortOrder};
use crate::db::model::types::ForeignKeyAction;
use crate::db::model::{EnumType, Sequence, Table, View};
use crate::db::schema::{
    ColumnName, ConstraintName, IndexName, SchemaName, SequenceName, TableName, TypeName,
};

use super::{RenderConfig, RenderedOperation, Renderer};

/// PostgreSQL SQL renderer.
pub struct PostgresRenderer {
    config: RenderConfig,
}

impl PostgresRenderer {
    /// Create a new PostgreSQL renderer with the given configuration.
    pub fn new(config: RenderConfig) -> Self {
        Self { config }
    }

    /// Quote an identifier according to the configuration.
    fn quote(&self, name: &str) -> String {
        self.config.quoting.quote(name)
    }

    /// Format a qualified name (schema.object).
    fn qualified(&self, schema: &SchemaName, name: &str) -> String {
        self.config.quoting.qualified(schema.as_ref(), name)
    }

    // =========================================================================
    // Table Rendering
    // =========================================================================

    fn render_create_table(&self, schema: &SchemaName, table: &Table) -> RenderedOperation {
        let mut sql = String::new();

        // CREATE TABLE
        sql.push_str("CREATE TABLE ");
        if self.config.if_not_exists {
            sql.push_str("IF NOT EXISTS ");
        }
        sql.push_str(&self.qualified(schema, table.name.as_ref()));
        sql.push_str(" (\n");

        // Columns
        let column_defs: Vec<String> = table
            .columns
            .iter()
            .map(|col| self.render_column_definition(col))
            .collect();
        sql.push_str(&column_defs.join(",\n"));

        // Inline constraints (PK, UNIQUE, CHECK - not FK or Exclusion)
        for constraint in &table.constraints {
            if let Some(inline) = self.render_inline_constraint(constraint) {
                sql.push_str(",\n");
                sql.push_str(&inline);
            }
        }

        sql.push_str("\n)");

        // Rollback
        let rollback = if self.config.generate_rollback {
            let mut drop_sql = String::from("DROP TABLE ");
            if self.config.if_exists {
                drop_sql.push_str("IF EXISTS ");
            }
            drop_sql.push_str(&self.qualified(schema, table.name.as_ref()));
            if self.config.cascade {
                drop_sql.push_str(" CASCADE");
            }
            Some(vec![drop_sql])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!("Create table {}.{}", schema.as_ref(), table.name.as_ref()),
        }
    }

    fn render_drop_table(&self, schema: &SchemaName, name: &TableName) -> RenderedOperation {
        let mut sql = String::from("DROP TABLE ");
        if self.config.if_exists {
            sql.push_str("IF EXISTS ");
        }
        sql.push_str(&self.qualified(schema, name.as_ref()));
        if self.config.cascade {
            sql.push_str(" CASCADE");
        }

        // Note: rollback for DROP TABLE would require the full table definition
        RenderedOperation::forward_only(
            vec![sql],
            format!("Drop table {}.{}", schema.as_ref(), name.as_ref()),
        )
    }

    fn render_rename_table(
        &self,
        schema: &SchemaName,
        from: &TableName,
        to: &TableName,
    ) -> RenderedOperation {
        let sql = format!(
            "ALTER TABLE {} RENAME TO {}",
            self.qualified(schema, from.as_ref()),
            self.quote(to.as_ref())
        );

        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "ALTER TABLE {} RENAME TO {}",
                self.qualified(schema, to.as_ref()),
                self.quote(from.as_ref())
            )])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Rename table {}.{} to {}",
                schema.as_ref(),
                from.as_ref(),
                to.as_ref()
            ),
        }
    }

    // =========================================================================
    // Column Rendering
    // =========================================================================

    fn render_column_definition(&self, column: &Column) -> String {
        let mut def = format!(
            "    {} {}",
            self.quote(column.name.as_ref()),
            &column.type_info.formatted
        );

        // Collation (only if non-default)
        if column.collation.name.as_ref() != "default" {
            def.push_str(&format!(
                " COLLATE {}",
                self.config.quoting.qualified(
                    column.collation.schema.as_ref(),
                    column.collation.name.as_ref()
                )
            ));
        }

        // NOT NULL
        if !column.is_nullable {
            def.push_str(" NOT NULL");
        }

        // Default
        if let Some(ref default) = column.default {
            def.push_str(&format!(" DEFAULT {}", default.as_ref()));
        }

        // Generated
        if let Some(ref generated) = column.generated {
            def.push_str(&format!(
                " GENERATED ALWAYS AS ({}) {}",
                generated.expression.as_ref(),
                generated.storage.as_sql()
            ));
        }

        // Identity
        if let Some(ref identity) = column.identity {
            def.push_str(&format!(" {}", identity.as_sql()));
        }

        def
    }

    fn render_add_column(
        &self,
        schema: &SchemaName,
        table: &TableName,
        column: &Column,
    ) -> RenderedOperation {
        let col_def = self.render_column_definition(column);
        let sql = format!(
            "ALTER TABLE {} ADD COLUMN{}",
            self.qualified(schema, table.as_ref()),
            &col_def[4..] // Remove leading indent
        );

        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "ALTER TABLE {} DROP COLUMN {}",
                self.qualified(schema, table.as_ref()),
                self.quote(column.name.as_ref())
            )])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Add column {}.{}.{}",
                schema.as_ref(),
                table.as_ref(),
                column.name.as_ref()
            ),
        }
    }

    fn render_drop_column(
        &self,
        schema: &SchemaName,
        table: &TableName,
        name: &ColumnName,
    ) -> RenderedOperation {
        let mut sql = format!(
            "ALTER TABLE {} DROP COLUMN ",
            self.qualified(schema, table.as_ref())
        );
        if self.config.if_exists {
            sql.push_str("IF EXISTS ");
        }
        sql.push_str(&self.quote(name.as_ref()));
        if self.config.cascade {
            sql.push_str(" CASCADE");
        }

        // Note: rollback for DROP COLUMN would require the full column definition
        RenderedOperation::forward_only(
            vec![sql],
            format!(
                "Drop column {}.{}.{}",
                schema.as_ref(),
                table.as_ref(),
                name.as_ref()
            ),
        )
    }

    fn render_rename_column(
        &self,
        schema: &SchemaName,
        table: &TableName,
        from: &ColumnName,
        to: &ColumnName,
    ) -> RenderedOperation {
        let sql = format!(
            "ALTER TABLE {} RENAME COLUMN {} TO {}",
            self.qualified(schema, table.as_ref()),
            self.quote(from.as_ref()),
            self.quote(to.as_ref())
        );

        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "ALTER TABLE {} RENAME COLUMN {} TO {}",
                self.qualified(schema, table.as_ref()),
                self.quote(to.as_ref()),
                self.quote(from.as_ref())
            )])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Rename column {}.{}.{} to {}",
                schema.as_ref(),
                table.as_ref(),
                from.as_ref(),
                to.as_ref()
            ),
        }
    }

    fn render_alter_column(
        &self,
        schema: &SchemaName,
        table: &TableName,
        name: &ColumnName,
        changes: &ColumnChanges,
    ) -> RenderedOperation {
        let mut forward = Vec::new();
        let mut rollback = Vec::new();
        let table_ref = self.qualified(schema, table.as_ref());
        let col_name = self.quote(name.as_ref());

        // Type change
        if let Some(ref type_change) = changes.set_type {
            forward.push(self.render_alter_column_type(&table_ref, &col_name, type_change));
            // Rollback for type change would require the original type
        }

        // NOT NULL change
        if let Some(not_null) = changes.set_not_null {
            if not_null {
                forward.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL",
                    table_ref, col_name
                ));
                if self.config.generate_rollback {
                    rollback.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL",
                        table_ref, col_name
                    ));
                }
            } else {
                forward.push(format!(
                    "ALTER TABLE {} ALTER COLUMN {} DROP NOT NULL",
                    table_ref, col_name
                ));
                if self.config.generate_rollback {
                    rollback.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} SET NOT NULL",
                        table_ref, col_name
                    ));
                }
            }
        }

        // Default change
        if let Some(ref default_change) = changes.set_default {
            match default_change {
                DefaultChange::Set(expr) => {
                    forward.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {}",
                        table_ref,
                        col_name,
                        expr.as_ref()
                    ));
                    if self.config.generate_rollback {
                        rollback.push(format!(
                            "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT",
                            table_ref, col_name
                        ));
                    }
                }
                DefaultChange::Drop => {
                    forward.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} DROP DEFAULT",
                        table_ref, col_name
                    ));
                    // Rollback would require the original default
                }
            }
        }

        // Identity change
        if let Some(ref identity_change) = changes.set_identity {
            match identity_change {
                IdentityChange::Add(kind) => {
                    let kind_sql = match kind {
                        crate::db::model::column::IdentityKind::Always => "ALWAYS",
                        crate::db::model::column::IdentityKind::ByDefault => "BY DEFAULT",
                    };
                    forward.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} ADD GENERATED {} AS IDENTITY",
                        table_ref, col_name, kind_sql
                    ));
                    if self.config.generate_rollback {
                        rollback.push(format!(
                            "ALTER TABLE {} ALTER COLUMN {} DROP IDENTITY",
                            table_ref, col_name
                        ));
                    }
                }
                IdentityChange::Drop => {
                    forward.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} DROP IDENTITY IF EXISTS",
                        table_ref, col_name
                    ));
                    // Rollback would require the original identity kind
                }
            }
        }

        // Generated column change
        if let Some(ref generated_change) = changes.set_generated {
            match generated_change {
                GeneratedChange::Set(generated) => {
                    // PostgreSQL doesn't support ALTER COLUMN to add GENERATED
                    // This would need DROP COLUMN + ADD COLUMN
                    forward.push(format!(
                        "-- Cannot add GENERATED to existing column; need DROP + ADD\n\
                         -- ALTER TABLE {} DROP COLUMN {};\n\
                         -- ALTER TABLE {} ADD COLUMN {} ... GENERATED ALWAYS AS ({}) STORED",
                        table_ref,
                        col_name,
                        table_ref,
                        col_name,
                        generated.expression.as_ref()
                    ));
                }
                GeneratedChange::Drop => {
                    forward.push(format!(
                        "ALTER TABLE {} ALTER COLUMN {} DROP EXPRESSION",
                        table_ref, col_name
                    ));
                }
            }
        }

        RenderedOperation {
            forward,
            rollback: if self.config.generate_rollback && !rollback.is_empty() {
                Some(rollback)
            } else {
                None
            },
            description: format!(
                "Alter column {}.{}.{}",
                schema.as_ref(),
                table.as_ref(),
                name.as_ref()
            ),
        }
    }

    fn render_alter_column_type(
        &self,
        table_ref: &str,
        col_name: &str,
        type_change: &SetColumnType,
    ) -> String {
        let mut sql = format!(
            "ALTER TABLE {} ALTER COLUMN {} TYPE {}",
            table_ref, col_name, type_change.type_info.formatted
        );

        if let Some(ref using) = type_change.using {
            sql.push_str(&format!(" USING {}", using.as_ref()));
        }

        sql
    }

    // =========================================================================
    // Constraint Rendering
    // =========================================================================

    fn render_inline_constraint(&self, constraint: &Constraint) -> Option<String> {
        match &constraint.kind {
            ConstraintKind::PrimaryKey(pk) => {
                let cols: Vec<_> = pk.columns.iter().map(|c| self.quote(c.as_ref())).collect();
                Some(format!(
                    "    CONSTRAINT {} PRIMARY KEY ({})",
                    self.quote(constraint.name.as_ref()),
                    cols.join(", ")
                ))
            }
            ConstraintKind::Unique(uq) => {
                let cols: Vec<_> = uq.columns.iter().map(|c| self.quote(c.as_ref())).collect();
                let nulls = if uq.nulls_not_distinct {
                    " NULLS NOT DISTINCT"
                } else {
                    ""
                };
                Some(format!(
                    "    CONSTRAINT {} UNIQUE{} ({})",
                    self.quote(constraint.name.as_ref()),
                    nulls,
                    cols.join(", ")
                ))
            }
            ConstraintKind::Check(ck) => {
                let no_inherit = if ck.is_no_inherit { " NO INHERIT" } else { "" };
                Some(format!(
                    "    CONSTRAINT {} CHECK ({}){}",
                    self.quote(constraint.name.as_ref()),
                    ck.expression.as_ref(),
                    no_inherit
                ))
            }
            // FK and Exclusion are added separately via ALTER TABLE
            ConstraintKind::ForeignKey(_) | ConstraintKind::Exclusion(_) => None,
        }
    }

    fn render_add_constraint(
        &self,
        schema: &SchemaName,
        table: &TableName,
        constraint: &Constraint,
    ) -> RenderedOperation {
        let table_ref = self.qualified(schema, table.as_ref());
        let constraint_sql = self.render_constraint_definition(constraint);

        let sql = format!(
            "ALTER TABLE {} ADD CONSTRAINT {} {}",
            table_ref,
            self.quote(constraint.name.as_ref()),
            constraint_sql
        );

        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "ALTER TABLE {} DROP CONSTRAINT {}",
                table_ref,
                self.quote(constraint.name.as_ref())
            )])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Add constraint {} on {}.{}",
                constraint.name.as_ref(),
                schema.as_ref(),
                table.as_ref()
            ),
        }
    }

    fn render_constraint_definition(&self, constraint: &Constraint) -> String {
        match &constraint.kind {
            ConstraintKind::PrimaryKey(pk) => {
                let cols: Vec<_> = pk.columns.iter().map(|c| self.quote(c.as_ref())).collect();
                format!("PRIMARY KEY ({})", cols.join(", "))
            }
            ConstraintKind::Unique(uq) => {
                let cols: Vec<_> = uq.columns.iter().map(|c| self.quote(c.as_ref())).collect();
                let nulls = if uq.nulls_not_distinct {
                    "NULLS NOT DISTINCT "
                } else {
                    ""
                };
                format!("UNIQUE {}({})", nulls, cols.join(", "))
            }
            ConstraintKind::Check(ck) => {
                let no_inherit = if ck.is_no_inherit { " NO INHERIT" } else { "" };
                format!("CHECK ({}){}", ck.expression.as_ref(), no_inherit)
            }
            ConstraintKind::ForeignKey(fk) => {
                let cols: Vec<_> = fk.columns.iter().map(|c| self.quote(c.as_ref())).collect();
                let ref_cols: Vec<_> = fk
                    .referenced_columns
                    .iter()
                    .map(|c| self.quote(c.as_ref()))
                    .collect();

                let mut sql = format!(
                    "FOREIGN KEY ({}) REFERENCES {} ({})",
                    cols.join(", "),
                    self.config.quoting.qualified(
                        fk.referenced_table.schema.as_ref(),
                        fk.referenced_table.name.as_ref()
                    ),
                    ref_cols.join(", ")
                );

                // ON DELETE
                if fk.on_delete != ForeignKeyAction::NoAction {
                    sql.push_str(&format!(" ON DELETE {}", fk.on_delete.as_sql()));
                }

                // ON UPDATE
                if fk.on_update != ForeignKeyAction::NoAction {
                    sql.push_str(&format!(" ON UPDATE {}", fk.on_update.as_sql()));
                }

                // DEFERRABLE
                if fk.is_deferrable {
                    sql.push_str(" DEFERRABLE");
                    if fk.is_initially_deferred {
                        sql.push_str(" INITIALLY DEFERRED");
                    }
                }

                sql
            }
            ConstraintKind::Exclusion(ex) => {
                let elements: Vec<String> = ex
                    .elements
                    .iter()
                    .map(|e| format!("({}) WITH {}", e.expression.as_ref(), &e.operator))
                    .collect();

                let mut sql = format!(
                    "EXCLUDE USING {} ({})",
                    ex.index_method.as_str(),
                    elements.join(", ")
                );

                if let Some(ref pred) = ex.predicate {
                    sql.push_str(&format!(" WHERE ({})", pred.as_ref()));
                }

                sql
            }
        }
    }

    fn render_drop_constraint(
        &self,
        schema: &SchemaName,
        table: &TableName,
        name: &ConstraintName,
    ) -> RenderedOperation {
        let mut sql = format!(
            "ALTER TABLE {} DROP CONSTRAINT ",
            self.qualified(schema, table.as_ref())
        );
        if self.config.if_exists {
            sql.push_str("IF EXISTS ");
        }
        sql.push_str(&self.quote(name.as_ref()));
        if self.config.cascade {
            sql.push_str(" CASCADE");
        }

        // Note: rollback would require the full constraint definition
        RenderedOperation::forward_only(
            vec![sql],
            format!(
                "Drop constraint {} from {}.{}",
                name.as_ref(),
                schema.as_ref(),
                table.as_ref()
            ),
        )
    }

    fn render_rename_constraint(
        &self,
        schema: &SchemaName,
        table: &TableName,
        from: &ConstraintName,
        to: &ConstraintName,
    ) -> RenderedOperation {
        let sql = format!(
            "ALTER TABLE {} RENAME CONSTRAINT {} TO {}",
            self.qualified(schema, table.as_ref()),
            self.quote(from.as_ref()),
            self.quote(to.as_ref())
        );

        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "ALTER TABLE {} RENAME CONSTRAINT {} TO {}",
                self.qualified(schema, table.as_ref()),
                self.quote(to.as_ref()),
                self.quote(from.as_ref())
            )])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Rename constraint {}.{}.{} to {}",
                schema.as_ref(),
                table.as_ref(),
                from.as_ref(),
                to.as_ref()
            ),
        }
    }

    // =========================================================================
    // Index Rendering
    // =========================================================================

    fn render_create_index(
        &self,
        schema: &SchemaName,
        table: &TableName,
        index: &Index,
        concurrently: bool,
    ) -> RenderedOperation {
        let mut sql = String::from("CREATE ");
        if index.is_unique {
            sql.push_str("UNIQUE ");
        }
        sql.push_str("INDEX ");
        if concurrently {
            sql.push_str("CONCURRENTLY ");
        }
        if self.config.if_not_exists {
            sql.push_str("IF NOT EXISTS ");
        }
        sql.push_str(&self.quote(index.name.as_ref()));
        sql.push_str(" ON ");
        sql.push_str(&self.qualified(schema, table.as_ref()));

        // Index method (if not btree)
        if index.method != crate::db::model::types::IndexMethod::BTree {
            sql.push_str(&format!(" USING {}", index.method.as_str()));
        }

        // Columns
        sql.push_str(" (");
        let col_defs: Vec<String> = index
            .columns
            .iter()
            .map(|c| self.render_index_column(c))
            .collect();
        sql.push_str(&col_defs.join(", "));
        sql.push(')');

        // Predicate (partial index)
        if let Some(ref pred) = index.predicate {
            sql.push_str(&format!(" WHERE {}", pred.as_ref()));
        }

        let rollback = if self.config.generate_rollback {
            let mut drop = String::from("DROP INDEX ");
            if concurrently {
                drop.push_str("CONCURRENTLY ");
            }
            if self.config.if_exists {
                drop.push_str("IF EXISTS ");
            }
            drop.push_str(&self.qualified(schema, index.name.as_ref()));
            Some(vec![drop])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Create index {} on {}.{}",
                index.name.as_ref(),
                schema.as_ref(),
                table.as_ref()
            ),
        }
    }

    fn render_index_column(&self, col: &IndexColumn) -> String {
        let mut sql = if let Some(ref name) = col.column {
            self.quote(name.as_ref())
        } else if let Some(ref expr) = col.expression {
            format!("({})", expr.as_ref())
        } else {
            "???".to_string()
        };

        // Sort order
        match col.order {
            SortOrder::Ascending => {} // Default, don't output
            SortOrder::Descending => sql.push_str(" DESC"),
        }

        // Nulls order
        match (col.order, col.nulls) {
            // Default: ASC NULLS LAST, DESC NULLS FIRST - don't output
            (SortOrder::Ascending, NullsOrder::Last) => {}
            (SortOrder::Descending, NullsOrder::First) => {}
            // Non-default: output explicitly
            (_, NullsOrder::First) => sql.push_str(" NULLS FIRST"),
            (_, NullsOrder::Last) => sql.push_str(" NULLS LAST"),
        }

        sql
    }

    fn render_drop_index(
        &self,
        schema: &SchemaName,
        name: &IndexName,
        concurrently: bool,
    ) -> RenderedOperation {
        let mut sql = String::from("DROP INDEX ");
        if concurrently {
            sql.push_str("CONCURRENTLY ");
        }
        if self.config.if_exists {
            sql.push_str("IF EXISTS ");
        }
        sql.push_str(&self.qualified(schema, name.as_ref()));
        if self.config.cascade {
            sql.push_str(" CASCADE");
        }

        // Note: rollback would require the full index definition
        RenderedOperation::forward_only(
            vec![sql],
            format!("Drop index {}.{}", schema.as_ref(), name.as_ref()),
        )
    }

    fn render_rename_index(
        &self,
        schema: &SchemaName,
        from: &IndexName,
        to: &IndexName,
    ) -> RenderedOperation {
        let sql = format!(
            "ALTER INDEX {} RENAME TO {}",
            self.qualified(schema, from.as_ref()),
            self.quote(to.as_ref())
        );

        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "ALTER INDEX {} RENAME TO {}",
                self.qualified(schema, to.as_ref()),
                self.quote(from.as_ref())
            )])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Rename index {}.{} to {}",
                schema.as_ref(),
                from.as_ref(),
                to.as_ref()
            ),
        }
    }

    // =========================================================================
    // Enum Rendering
    // =========================================================================

    fn render_create_enum(&self, schema: &SchemaName, enum_type: &EnumType) -> RenderedOperation {
        let values: Vec<String> = enum_type
            .values
            .iter()
            .map(|v| format!("'{}'", v.replace('\'', "''")))
            .collect();

        let sql = format!(
            "CREATE TYPE {} AS ENUM ({})",
            self.qualified(schema, enum_type.name.as_ref()),
            values.join(", ")
        );

        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "DROP TYPE {}",
                self.qualified(schema, enum_type.name.as_ref())
            )])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Create enum type {}.{}",
                schema.as_ref(),
                enum_type.name.as_ref()
            ),
        }
    }

    fn render_drop_enum(&self, schema: &SchemaName, name: &TypeName) -> RenderedOperation {
        let mut sql = String::from("DROP TYPE ");
        if self.config.if_exists {
            sql.push_str("IF EXISTS ");
        }
        sql.push_str(&self.qualified(schema, name.as_ref()));
        if self.config.cascade {
            sql.push_str(" CASCADE");
        }

        // Note: rollback would require the full enum definition
        RenderedOperation::forward_only(
            vec![sql],
            format!("Drop enum type {}.{}", schema.as_ref(), name.as_ref()),
        )
    }

    fn render_rename_enum(
        &self,
        schema: &SchemaName,
        from: &TypeName,
        to: &TypeName,
    ) -> RenderedOperation {
        let sql = format!(
            "ALTER TYPE {} RENAME TO {}",
            self.qualified(schema, from.as_ref()),
            self.quote(to.as_ref())
        );

        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "ALTER TYPE {} RENAME TO {}",
                self.qualified(schema, to.as_ref()),
                self.quote(from.as_ref())
            )])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Rename enum type {}.{} to {}",
                schema.as_ref(),
                from.as_ref(),
                to.as_ref()
            ),
        }
    }

    fn render_add_enum_value(
        &self,
        schema: &SchemaName,
        enum_name: &TypeName,
        value: &str,
        position: &EnumValuePosition,
    ) -> RenderedOperation {
        let escaped_value = value.replace('\'', "''");
        let mut sql = format!(
            "ALTER TYPE {} ADD VALUE '{}'",
            self.qualified(schema, enum_name.as_ref()),
            escaped_value
        );

        match position {
            EnumValuePosition::End => {}
            EnumValuePosition::Before(before) => {
                sql.push_str(&format!(" BEFORE '{}'", before.replace('\'', "''")));
            }
            EnumValuePosition::After(after) => {
                sql.push_str(&format!(" AFTER '{}'", after.replace('\'', "''")));
            }
        }

        // Note: PostgreSQL doesn't support removing enum values, so no rollback
        RenderedOperation::forward_only(
            vec![sql],
            format!(
                "Add value '{}' to enum {}.{}",
                value,
                schema.as_ref(),
                enum_name.as_ref()
            ),
        )
    }

    // =========================================================================
    // Sequence Rendering
    // =========================================================================

    fn render_create_sequence(
        &self,
        schema: &SchemaName,
        sequence: &Sequence,
    ) -> RenderedOperation {
        let mut sql = String::from("CREATE SEQUENCE ");
        if self.config.if_not_exists {
            sql.push_str("IF NOT EXISTS ");
        }
        sql.push_str(&self.qualified(schema, sequence.name.as_ref()));

        // Data type
        sql.push_str(&format!(" AS {}", sequence.data_type.formatted));

        // Start, increment, min, max
        sql.push_str(&format!(" START WITH {}", sequence.start_value));
        sql.push_str(&format!(" INCREMENT BY {}", sequence.increment));
        sql.push_str(&format!(" MINVALUE {}", sequence.min_value));
        sql.push_str(&format!(" MAXVALUE {}", sequence.max_value));

        // Cache
        sql.push_str(&format!(" CACHE {}", sequence.cache_size));

        // Cycle
        if sequence.is_cyclic {
            sql.push_str(" CYCLE");
        } else {
            sql.push_str(" NO CYCLE");
        }

        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "DROP SEQUENCE {}",
                self.qualified(schema, sequence.name.as_ref())
            )])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Create sequence {}.{}",
                schema.as_ref(),
                sequence.name.as_ref()
            ),
        }
    }

    fn render_drop_sequence(&self, schema: &SchemaName, name: &SequenceName) -> RenderedOperation {
        let mut sql = String::from("DROP SEQUENCE ");
        if self.config.if_exists {
            sql.push_str("IF EXISTS ");
        }
        sql.push_str(&self.qualified(schema, name.as_ref()));
        if self.config.cascade {
            sql.push_str(" CASCADE");
        }

        // Note: rollback would require the full sequence definition
        RenderedOperation::forward_only(
            vec![sql],
            format!("Drop sequence {}.{}", schema.as_ref(), name.as_ref()),
        )
    }

    fn render_rename_sequence(
        &self,
        schema: &SchemaName,
        from: &SequenceName,
        to: &SequenceName,
    ) -> RenderedOperation {
        let sql = format!(
            "ALTER SEQUENCE {} RENAME TO {}",
            self.qualified(schema, from.as_ref()),
            self.quote(to.as_ref())
        );

        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "ALTER SEQUENCE {} RENAME TO {}",
                self.qualified(schema, to.as_ref()),
                self.quote(from.as_ref())
            )])
        } else {
            None
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Rename sequence {}.{} to {}",
                schema.as_ref(),
                from.as_ref(),
                to.as_ref()
            ),
        }
    }

    fn render_alter_sequence(
        &self,
        schema: &SchemaName,
        name: &SequenceName,
        changes: &SequenceChanges,
    ) -> RenderedOperation {
        let mut clauses = Vec::new();
        let seq_ref = self.qualified(schema, name.as_ref());

        if let Some(ref dt) = changes.data_type {
            clauses.push(format!("AS {}", dt.formatted));
        }
        if let Some(inc) = changes.increment {
            clauses.push(format!("INCREMENT BY {}", inc));
        }
        if let Some(min) = changes.min_value {
            clauses.push(format!("MINVALUE {}", min));
        }
        if let Some(max) = changes.max_value {
            clauses.push(format!("MAXVALUE {}", max));
        }
        if let Some(start) = changes.start_value {
            clauses.push(format!("START WITH {}", start));
        }
        if let Some(cache) = changes.cache_size {
            clauses.push(format!("CACHE {}", cache));
        }
        if let Some(cyclic) = changes.is_cyclic {
            if cyclic {
                clauses.push("CYCLE".to_string());
            } else {
                clauses.push("NO CYCLE".to_string());
            }
        }

        let sql = format!("ALTER SEQUENCE {} {}", seq_ref, clauses.join(" "));

        // Note: rollback would require the original values
        RenderedOperation::forward_only(
            vec![sql],
            format!("Alter sequence {}.{}", schema.as_ref(), name.as_ref()),
        )
    }

    // =========================================================================
    // View Rendering
    // =========================================================================

    fn render_create_view(&self, schema: &SchemaName, view: &View) -> RenderedOperation {
        let keyword = if view.is_materialized {
            "MATERIALIZED VIEW"
        } else {
            "VIEW"
        };

        let sql = format!(
            "CREATE {} {} AS {}",
            keyword,
            self.qualified(schema, view.name.as_ref()),
            view.definition.as_ref()
        );

        let rollback = if self.config.generate_rollback {
            let drop_keyword = if view.is_materialized {
                "MATERIALIZED VIEW"
            } else {
                "VIEW"
            };
            Some(vec![format!(
                "DROP {} {}",
                drop_keyword,
                self.qualified(schema, view.name.as_ref())
            )])
        } else {
            None
        };

        let kind = if view.is_materialized {
            "materialized view"
        } else {
            "view"
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!("Create {} {}.{}", kind, schema.as_ref(), view.name.as_ref()),
        }
    }

    fn render_drop_view(
        &self,
        schema: &SchemaName,
        name: &TableName,
        is_materialized: bool,
    ) -> RenderedOperation {
        let keyword = if is_materialized {
            "MATERIALIZED VIEW"
        } else {
            "VIEW"
        };

        let mut sql = format!("DROP {} ", keyword);
        if self.config.if_exists {
            sql.push_str("IF EXISTS ");
        }
        sql.push_str(&self.qualified(schema, name.as_ref()));
        if self.config.cascade {
            sql.push_str(" CASCADE");
        }

        let kind = if is_materialized {
            "materialized view"
        } else {
            "view"
        };

        // Note: rollback would require the full view definition
        RenderedOperation::forward_only(
            vec![sql],
            format!("Drop {} {}.{}", kind, schema.as_ref(), name.as_ref()),
        )
    }

    fn render_rename_view(
        &self,
        schema: &SchemaName,
        from: &TableName,
        to: &TableName,
        is_materialized: bool,
    ) -> RenderedOperation {
        let keyword = if is_materialized {
            "MATERIALIZED VIEW"
        } else {
            "VIEW"
        };

        let sql = format!(
            "ALTER {} {} RENAME TO {}",
            keyword,
            self.qualified(schema, from.as_ref()),
            self.quote(to.as_ref())
        );

        let rollback = if self.config.generate_rollback {
            Some(vec![format!(
                "ALTER {} {} RENAME TO {}",
                keyword,
                self.qualified(schema, to.as_ref()),
                self.quote(from.as_ref())
            )])
        } else {
            None
        };

        let kind = if is_materialized {
            "materialized view"
        } else {
            "view"
        };

        RenderedOperation {
            forward: vec![sql],
            rollback,
            description: format!(
                "Rename {} {}.{} to {}",
                kind,
                schema.as_ref(),
                from.as_ref(),
                to.as_ref()
            ),
        }
    }

    fn render_replace_view(&self, schema: &SchemaName, view: &View) -> RenderedOperation {
        // CREATE OR REPLACE only works for non-materialized views
        let sql = format!(
            "CREATE OR REPLACE VIEW {} AS {}",
            self.qualified(schema, view.name.as_ref()),
            view.definition.as_ref()
        );

        // Note: rollback would require the original view definition
        RenderedOperation::forward_only(
            vec![sql],
            format!("Replace view {}.{}", schema.as_ref(), view.name.as_ref()),
        )
    }

    fn render_refresh_materialized_view(
        &self,
        schema: &SchemaName,
        name: &TableName,
        concurrently: bool,
    ) -> RenderedOperation {
        let mut sql = String::from("REFRESH MATERIALIZED VIEW ");
        if concurrently {
            sql.push_str("CONCURRENTLY ");
        }
        sql.push_str(&self.qualified(schema, name.as_ref()));

        RenderedOperation::forward_only(
            vec![sql],
            format!(
                "Refresh materialized view {}.{}",
                schema.as_ref(),
                name.as_ref()
            ),
        )
    }

    // =========================================================================
    // Comment Rendering
    // =========================================================================

    fn render_set_comment(
        &self,
        target: &CommentTarget,
        comment: &Option<String>,
    ) -> RenderedOperation {
        let (object_type, object_ref) = match target {
            CommentTarget::Schema(name) => ("SCHEMA".to_string(), self.quote(name.as_ref())),
            CommentTarget::Table { schema, table } => {
                ("TABLE".to_string(), self.qualified(schema, table.as_ref()))
            }
            CommentTarget::Column {
                schema,
                table,
                column,
            } => (
                "COLUMN".to_string(),
                format!(
                    "{}.{}",
                    self.qualified(schema, table.as_ref()),
                    self.quote(column.as_ref())
                ),
            ),
            CommentTarget::Index { schema, index } => {
                ("INDEX".to_string(), self.qualified(schema, index.as_ref()))
            }
            CommentTarget::Constraint {
                schema,
                table,
                constraint,
            } => (
                format!("CONSTRAINT {} ON", self.quote(constraint.as_ref())),
                self.qualified(schema, table.as_ref()),
            ),
            CommentTarget::Sequence { schema, sequence } => (
                "SEQUENCE".to_string(),
                self.qualified(schema, sequence.as_ref()),
            ),
            CommentTarget::Type { schema, type_name } => (
                "TYPE".to_string(),
                self.qualified(schema, type_name.as_ref()),
            ),
            CommentTarget::View { schema, view } => {
                ("VIEW".to_string(), self.qualified(schema, view.as_ref()))
            }
        };

        let comment_value = match comment {
            Some(c) => format!("'{}'", c.replace('\'', "''")),
            None => "NULL".to_string(),
        };

        let sql = format!(
            "COMMENT ON {} {} IS {}",
            object_type, object_ref, comment_value
        );

        let action = if comment.is_some() { "Set" } else { "Remove" };

        // Note: rollback would require the original comment
        RenderedOperation::forward_only(
            vec![sql],
            format!("{} comment on {}", action, target.description()),
        )
    }
}

impl Renderer for PostgresRenderer {
    fn render(&self, operation: &Operation) -> RenderedOperation {
        match operation {
            // Table operations
            Operation::CreateTable { schema, table } => self.render_create_table(schema, table),
            Operation::DropTable { schema, name } => self.render_drop_table(schema, name),
            Operation::RenameTable { schema, from, to } => {
                self.render_rename_table(schema, from, to)
            }

            // Column operations
            Operation::AddColumn {
                schema,
                table,
                column,
            } => self.render_add_column(schema, table, column),
            Operation::DropColumn {
                schema,
                table,
                name,
            } => self.render_drop_column(schema, table, name),
            Operation::RenameColumn {
                schema,
                table,
                from,
                to,
            } => self.render_rename_column(schema, table, from, to),
            Operation::AlterColumn {
                schema,
                table,
                name,
                changes,
            } => self.render_alter_column(schema, table, name, changes),

            // Constraint operations
            Operation::AddConstraint {
                schema,
                table,
                constraint,
            } => self.render_add_constraint(schema, table, constraint),
            Operation::DropConstraint {
                schema,
                table,
                name,
            } => self.render_drop_constraint(schema, table, name),
            Operation::RenameConstraint {
                schema,
                table,
                from,
                to,
            } => self.render_rename_constraint(schema, table, from, to),

            // Index operations
            Operation::CreateIndex {
                schema,
                table,
                index,
                concurrently,
            } => self.render_create_index(schema, table, index, *concurrently),
            Operation::DropIndex {
                schema,
                name,
                concurrently,
            } => self.render_drop_index(schema, name, *concurrently),
            Operation::RenameIndex { schema, from, to } => {
                self.render_rename_index(schema, from, to)
            }

            // Enum operations
            Operation::CreateEnum { schema, enum_type } => {
                self.render_create_enum(schema, enum_type)
            }
            Operation::DropEnum { schema, name } => self.render_drop_enum(schema, name),
            Operation::RenameEnum { schema, from, to } => self.render_rename_enum(schema, from, to),
            Operation::AddEnumValue {
                schema,
                enum_name,
                value,
                position,
            } => self.render_add_enum_value(schema, enum_name, value, position),

            // Sequence operations
            Operation::CreateSequence { schema, sequence } => {
                self.render_create_sequence(schema, sequence)
            }
            Operation::DropSequence { schema, name } => self.render_drop_sequence(schema, name),
            Operation::RenameSequence { schema, from, to } => {
                self.render_rename_sequence(schema, from, to)
            }
            Operation::AlterSequence {
                schema,
                name,
                changes,
            } => self.render_alter_sequence(schema, name, changes),

            // View operations
            Operation::CreateView { schema, view } => self.render_create_view(schema, view),
            Operation::DropView {
                schema,
                name,
                is_materialized,
            } => self.render_drop_view(schema, name, *is_materialized),
            Operation::RenameView {
                schema,
                from,
                to,
                is_materialized,
            } => self.render_rename_view(schema, from, to, *is_materialized),
            Operation::ReplaceView { schema, view } => self.render_replace_view(schema, view),
            Operation::RefreshMaterializedView {
                schema,
                name,
                concurrently,
            } => self.render_refresh_materialized_view(schema, name, *concurrently),

            // Comment operations
            Operation::SetComment { target, comment } => self.render_set_comment(target, comment),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::model::constraint::PrimaryKeyConstraint;
    use crate::db::model::types::TypeInfo;
    use crate::db::schema::Oid;

    fn test_schema() -> SchemaName {
        SchemaName::try_new("public".to_string()).unwrap()
    }

    fn test_table_name() -> TableName {
        TableName::try_new("users".to_string()).unwrap()
    }

    fn test_column() -> Column {
        Column {
            name: ColumnName::try_new("id".to_string()).unwrap(),
            position: 1,
            type_info: TypeInfo {
                name: TypeName::try_new("int4".to_string()).unwrap(),
                schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                formatted: "integer".to_string(),
                is_array: false,
            },
            is_nullable: false,
            default: None,
            generated: None,
            identity: None,
            collation: crate::db::model::types::QualifiedCollationName::new(
                SchemaName::try_new("pg_catalog".to_string()).unwrap(),
                crate::db::schema::CollationName::try_new("default".to_string()).unwrap(),
            ),
            comment: None,
        }
    }

    #[test]
    fn render_create_simple_table() {
        let renderer = PostgresRenderer::new(RenderConfig::default());
        let table = Table {
            oid: Oid::new(1),
            name: test_table_name(),
            kind: crate::db::model::TableKind::Regular,
            columns: vec![test_column()],
            constraints: vec![],
            indexes: vec![],
            comment: None,
        };

        let op = Operation::CreateTable {
            schema: test_schema(),
            table,
        };

        let rendered = renderer.render(&op);
        assert!(rendered.forward[0].contains("CREATE TABLE"));
        assert!(rendered.forward[0].contains("public.users"));
        assert!(rendered.forward[0].contains("id integer NOT NULL"));
        assert!(rendered.rollback.is_some());
        assert!(rendered.rollback.unwrap()[0].contains("DROP TABLE"));
    }

    #[test]
    fn render_drop_table() {
        let renderer = PostgresRenderer::new(RenderConfig {
            if_exists: true,
            cascade: true,
            ..Default::default()
        });

        let op = Operation::DropTable {
            schema: test_schema(),
            name: test_table_name(),
        };

        let rendered = renderer.render(&op);
        assert!(rendered.forward[0].contains("DROP TABLE IF EXISTS"));
        assert!(rendered.forward[0].contains("CASCADE"));
    }

    #[test]
    fn render_add_column() {
        let renderer = PostgresRenderer::new(RenderConfig::default());
        let column = test_column();

        let op = Operation::AddColumn {
            schema: test_schema(),
            table: test_table_name(),
            column,
        };

        let rendered = renderer.render(&op);
        assert!(rendered.forward[0].contains("ALTER TABLE"));
        assert!(rendered.forward[0].contains("ADD COLUMN"));
        assert!(rendered.forward[0].contains("id integer NOT NULL"));
    }

    #[test]
    fn render_alter_column_set_not_null() {
        let renderer = PostgresRenderer::new(RenderConfig::default());

        let op = Operation::AlterColumn {
            schema: test_schema(),
            table: test_table_name(),
            name: ColumnName::try_new("email".to_string()).unwrap(),
            changes: ColumnChanges {
                set_not_null: Some(true),
                ..Default::default()
            },
        };

        let rendered = renderer.render(&op);
        assert!(rendered.forward[0].contains("SET NOT NULL"));
        assert!(rendered.rollback.is_some());
        assert!(rendered.rollback.unwrap()[0].contains("DROP NOT NULL"));
    }

    #[test]
    fn render_create_index() {
        let renderer = PostgresRenderer::new(RenderConfig::default());
        let index = Index {
            oid: Oid::new(1),
            name: IndexName::try_new("users_email_idx".to_string()).unwrap(),
            method: crate::db::model::types::IndexMethod::BTree,
            is_unique: true,
            is_constraint_index: false,
            columns: vec![IndexColumn {
                column: Some(ColumnName::try_new("email".to_string()).unwrap()),
                expression: None,
                order: SortOrder::Ascending,
                nulls: NullsOrder::Last,
            }],
            predicate: None,
            comment: None,
        };

        let op = Operation::CreateIndex {
            schema: test_schema(),
            table: test_table_name(),
            index,
            concurrently: false,
        };

        let rendered = renderer.render(&op);
        assert!(rendered.forward[0].contains("CREATE UNIQUE INDEX"));
        assert!(rendered.forward[0].contains("users_email_idx"));
        assert!(rendered.forward[0].contains("ON public.users"));
    }

    #[test]
    fn render_create_enum() {
        let renderer = PostgresRenderer::new(RenderConfig::default());
        let enum_type = EnumType {
            oid: Oid::new(1),
            name: TypeName::try_new("status".to_string()).unwrap(),
            values: vec!["active".to_string(), "inactive".to_string()],
            comment: None,
        };

        let op = Operation::CreateEnum {
            schema: test_schema(),
            enum_type,
        };

        let rendered = renderer.render(&op);
        assert!(rendered.forward[0].contains("CREATE TYPE public.status AS ENUM"));
        assert!(rendered.forward[0].contains("'active'"));
        assert!(rendered.forward[0].contains("'inactive'"));
    }

    #[test]
    fn render_add_constraint_pk() {
        let renderer = PostgresRenderer::new(RenderConfig::default());
        let constraint = Constraint {
            name: ConstraintName::try_new("users_pkey".to_string()).unwrap(),
            kind: ConstraintKind::PrimaryKey(PrimaryKeyConstraint {
                columns: vec![ColumnName::try_new("id".to_string()).unwrap()],
                index_name: IndexName::try_new("users_pkey".to_string()).unwrap(),
            }),
            comment: None,
        };

        let op = Operation::AddConstraint {
            schema: test_schema(),
            table: test_table_name(),
            constraint,
        };

        let rendered = renderer.render(&op);
        assert!(rendered.forward[0].contains("ADD CONSTRAINT"));
        assert!(rendered.forward[0].contains("PRIMARY KEY"));
    }
}
