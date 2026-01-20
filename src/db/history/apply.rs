//! Application of operations to namespaces.
//!
//! This module provides the ability to transform a `Namespace` by applying
//! a sequence of `Operation`s, producing a new `Namespace` that reflects
//! the changes.
//!
//! This is the inverse of the diff/operation-collection pipeline:
//! - **Diff pipeline**: `Namespace` → `Namespace` → `NamespaceDiff` → `Vec<Operation>`
//! - **Apply pipeline**: `Namespace` + `Vec<Operation>` → `Namespace`
//!
//! Having both directions enables:
//! - **Snapshot testing**: Construct schemas programmatically, diff them, verify output
//! - **History reconstruction**: Walk a chain of migrations to rebuild schema state
//! - **Verification**: Apply operations, re-diff, ensure result is empty

use crate::db::migrate::{
    ColumnChanges, CommentTarget, DefaultChange, EnumValuePosition, GeneratedChange,
    IdentityChange, Operation, SequenceChanges,
};
use crate::db::model::{Constraint, EnumType, Index, Namespace, Sequence, Table, View};
use crate::db::schema::{
    ColumnName, ConstraintName, IndexName, Oid, SchemaName, SequenceName, TableName, TypeName,
};

use super::error::ApplyError;

impl Namespace {
    /// Applies a sequence of operations to this namespace, returning a new namespace.
    ///
    /// This is a pure function that does not modify `self`. Instead, it returns
    /// a new `Namespace` with all operations applied in order.
    ///
    /// # Arguments
    ///
    /// * `operations` - The operations to apply, in order
    ///
    /// # Returns
    ///
    /// A new `Namespace` with all operations applied, or an error if any
    /// operation cannot be applied (e.g., referencing a non-existent table).
    ///
    /// # Example
    ///
    /// ```ignore
    /// use tern::db::model::Namespace;
    /// use tern::db::migrate::Operation;
    ///
    /// let source = Namespace::empty("public");
    /// let ops = vec![Operation::CreateTable { ... }];
    /// let target = source.apply(&ops)?;
    /// ```
    pub fn apply(&self, operations: &[Operation]) -> Result<Namespace, ApplyError> {
        let mut result = self.clone();

        for operation in operations {
            result.apply_one(operation)?;
        }

        Ok(result)
    }

    /// Applies a single operation to this namespace, mutating it in place.
    fn apply_one(&mut self, operation: &Operation) -> Result<(), ApplyError> {
        match operation {
            // Enum operations
            Operation::CreateEnum { schema, enum_type } => {
                self.verify_schema(schema)?;
                self.create_enum(enum_type.clone())?;
            }
            Operation::DropEnum { schema, name } => {
                self.verify_schema(schema)?;
                self.drop_enum(name)?;
            }
            Operation::RenameEnum { schema, from, to } => {
                self.verify_schema(schema)?;
                self.rename_enum(from, to)?;
            }
            Operation::AddEnumValue {
                schema,
                enum_name,
                value,
                position,
            } => {
                self.verify_schema(schema)?;
                self.add_enum_value(enum_name, value, position)?;
            }

            // Sequence operations
            Operation::CreateSequence { schema, sequence } => {
                self.verify_schema(schema)?;
                self.create_sequence(sequence.clone())?;
            }
            Operation::DropSequence { schema, name } => {
                self.verify_schema(schema)?;
                self.drop_sequence(name)?;
            }
            Operation::RenameSequence { schema, from, to } => {
                self.verify_schema(schema)?;
                self.rename_sequence(from, to)?;
            }
            Operation::AlterSequence {
                schema,
                name,
                changes,
            } => {
                self.verify_schema(schema)?;
                self.alter_sequence(name, changes)?;
            }

            // Table operations
            Operation::CreateTable { schema, table } => {
                self.verify_schema(schema)?;
                self.create_table(table.clone())?;
            }
            Operation::DropTable { schema, name } => {
                self.verify_schema(schema)?;
                self.drop_table(name)?;
            }
            Operation::RenameTable { schema, from, to } => {
                self.verify_schema(schema)?;
                self.rename_table(from, to)?;
            }

            // Column operations
            Operation::AddColumn {
                schema,
                table,
                column,
            } => {
                self.verify_schema(schema)?;
                self.add_column(table, column.clone())?;
            }
            Operation::DropColumn {
                schema,
                table,
                name,
            } => {
                self.verify_schema(schema)?;
                self.drop_column(table, name)?;
            }
            Operation::RenameColumn {
                schema,
                table,
                from,
                to,
            } => {
                self.verify_schema(schema)?;
                self.rename_column(table, from, to)?;
            }
            Operation::AlterColumn {
                schema,
                table,
                name,
                changes,
            } => {
                self.verify_schema(schema)?;
                self.alter_column(table, name, changes)?;
            }

            // Constraint operations
            Operation::AddConstraint {
                schema,
                table,
                constraint,
            } => {
                self.verify_schema(schema)?;
                self.add_constraint(table, constraint.clone())?;
            }
            Operation::DropConstraint {
                schema,
                table,
                name,
            } => {
                self.verify_schema(schema)?;
                self.drop_constraint(table, name)?;
            }
            Operation::RenameConstraint {
                schema,
                table,
                from,
                to,
            } => {
                self.verify_schema(schema)?;
                self.rename_constraint(table, from, to)?;
            }

            // Index operations
            Operation::CreateIndex {
                schema,
                table,
                index,
                ..
            } => {
                self.verify_schema(schema)?;
                self.create_index(table, index.clone())?;
            }
            Operation::DropIndex { schema, name, .. } => {
                self.verify_schema(schema)?;
                self.drop_index(name)?;
            }
            Operation::RenameIndex { schema, from, to } => {
                self.verify_schema(schema)?;
                self.rename_index(from, to)?;
            }

            // View operations
            Operation::CreateView { schema, view } => {
                self.verify_schema(schema)?;
                self.create_view(view.clone())?;
            }
            Operation::DropView { schema, name, .. } => {
                self.verify_schema(schema)?;
                self.drop_view(name)?;
            }
            Operation::RenameView {
                schema, from, to, ..
            } => {
                self.verify_schema(schema)?;
                self.rename_view(from, to)?;
            }
            Operation::ReplaceView { schema, view } => {
                self.verify_schema(schema)?;
                self.replace_view(view.clone())?;
            }
            Operation::RefreshMaterializedView { .. } => {
                // RefreshMaterializedView doesn't change schema structure,
                // only data. This is a no-op for schema state.
            }

            // Comment operations
            Operation::SetComment { target, comment } => {
                self.set_comment(target, comment.clone())?;
            }
        }

        Ok(())
    }

    /// Verifies that an operation's schema matches this namespace.
    fn verify_schema(&self, schema: &SchemaName) -> Result<(), ApplyError> {
        if schema != &self.name {
            return Err(ApplyError::SchemaMismatch {
                target: schema.as_ref().to_string(),
                actual: self.name.as_ref().to_string(),
            });
        }
        Ok(())
    }

    /// Returns the schema name as a string, for use in error messages.
    fn schema_str(&self) -> String {
        self.name.as_ref().to_string()
    }

    // =========================================================================
    // Enum Operations
    // =========================================================================

    fn create_enum(&mut self, enum_type: EnumType) -> Result<(), ApplyError> {
        if self.enums.iter().any(|e| e.name == enum_type.name) {
            return Err(ApplyError::AlreadyExists {
                schema: self.schema_str(),
                name: enum_type.name.as_ref().to_string(),
                kind: "enum",
            });
        }
        self.enums.push(enum_type);
        Ok(())
    }

    fn drop_enum(&mut self, name: &TypeName) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let pos = self
            .enums
            .iter()
            .position(|e| &e.name == name)
            .ok_or_else(|| ApplyError::EnumNotFound {
                schema: schema_str,
                name: name.as_ref().to_string(),
            })?;
        self.enums.remove(pos);
        Ok(())
    }

    fn rename_enum(&mut self, from: &TypeName, to: &TypeName) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let enum_type = self
            .enums
            .iter_mut()
            .find(|e| &e.name == from)
            .ok_or_else(|| ApplyError::EnumNotFound {
                schema: schema_str,
                name: from.as_ref().to_string(),
            })?;
        enum_type.name = to.clone();
        Ok(())
    }

    fn add_enum_value(
        &mut self,
        enum_name: &TypeName,
        value: &str,
        position: &EnumValuePosition,
    ) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let enum_type = self
            .enums
            .iter_mut()
            .find(|e| &e.name == enum_name)
            .ok_or_else(|| ApplyError::EnumNotFound {
                schema: schema_str.clone(),
                name: enum_name.as_ref().to_string(),
            })?;

        match position {
            EnumValuePosition::End => {
                enum_type.values.push(value.to_string());
            }
            EnumValuePosition::Before(reference) => {
                let pos = enum_type
                    .values
                    .iter()
                    .position(|v| v == reference)
                    .ok_or_else(|| ApplyError::EnumValueNotFound {
                        schema: schema_str,
                        enum_name: enum_name.as_ref().to_string(),
                        reference: reference.clone(),
                    })?;
                enum_type.values.insert(pos, value.to_string());
            }
            EnumValuePosition::After(reference) => {
                let pos = enum_type
                    .values
                    .iter()
                    .position(|v| v == reference)
                    .ok_or_else(|| ApplyError::EnumValueNotFound {
                        schema: schema_str,
                        enum_name: enum_name.as_ref().to_string(),
                        reference: reference.clone(),
                    })?;
                enum_type.values.insert(pos + 1, value.to_string());
            }
        }

        Ok(())
    }

    // =========================================================================
    // Sequence Operations
    // =========================================================================

    fn create_sequence(&mut self, sequence: Sequence) -> Result<(), ApplyError> {
        if self.sequences.iter().any(|s| s.name == sequence.name) {
            return Err(ApplyError::AlreadyExists {
                schema: self.schema_str(),
                name: sequence.name.as_ref().to_string(),
                kind: "sequence",
            });
        }
        self.sequences.push(sequence);
        Ok(())
    }

    fn drop_sequence(&mut self, name: &SequenceName) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let pos = self
            .sequences
            .iter()
            .position(|s| &s.name == name)
            .ok_or_else(|| ApplyError::SequenceNotFound {
                schema: schema_str,
                name: name.as_ref().to_string(),
            })?;
        self.sequences.remove(pos);
        Ok(())
    }

    fn rename_sequence(
        &mut self,
        from: &SequenceName,
        to: &SequenceName,
    ) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let sequence = self
            .sequences
            .iter_mut()
            .find(|s| &s.name == from)
            .ok_or_else(|| ApplyError::SequenceNotFound {
                schema: schema_str,
                name: from.as_ref().to_string(),
            })?;
        sequence.name = to.clone();
        Ok(())
    }

    fn alter_sequence(
        &mut self,
        name: &SequenceName,
        changes: &SequenceChanges,
    ) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let sequence = self
            .sequences
            .iter_mut()
            .find(|s| &s.name == name)
            .ok_or_else(|| ApplyError::SequenceNotFound {
                schema: schema_str,
                name: name.as_ref().to_string(),
            })?;

        if let Some(ref data_type) = changes.data_type {
            sequence.data_type = data_type.clone();
        }
        if let Some(increment) = changes.increment {
            sequence.increment = increment;
        }
        if let Some(min_value) = changes.min_value {
            sequence.min_value = min_value;
        }
        if let Some(max_value) = changes.max_value {
            sequence.max_value = max_value;
        }
        if let Some(start_value) = changes.start_value {
            sequence.start_value = start_value;
        }
        if let Some(cache_size) = changes.cache_size {
            sequence.cache_size = cache_size;
        }
        if let Some(is_cyclic) = changes.is_cyclic {
            sequence.is_cyclic = is_cyclic;
        }

        Ok(())
    }

    // =========================================================================
    // Table Operations
    // =========================================================================

    fn create_table(&mut self, table: Table) -> Result<(), ApplyError> {
        if self.tables.iter().any(|t| t.name == table.name) {
            return Err(ApplyError::AlreadyExists {
                schema: self.schema_str(),
                name: table.name.as_ref().to_string(),
                kind: "table",
            });
        }
        self.tables.push(table);
        Ok(())
    }

    fn drop_table(&mut self, name: &TableName) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let pos = self
            .tables
            .iter()
            .position(|t| &t.name == name)
            .ok_or_else(|| ApplyError::TableNotFound {
                schema: schema_str,
                name: name.as_ref().to_string(),
            })?;
        self.tables.remove(pos);
        Ok(())
    }

    fn rename_table(&mut self, from: &TableName, to: &TableName) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let table = self
            .tables
            .iter_mut()
            .find(|t| &t.name == from)
            .ok_or_else(|| ApplyError::TableNotFound {
                schema: schema_str,
                name: from.as_ref().to_string(),
            })?;
        table.name = to.clone();
        Ok(())
    }

    // =========================================================================
    // Column Operations
    // =========================================================================

    fn add_column(
        &mut self,
        table_name: &TableName,
        column: crate::db::model::Column,
    ) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let table = self
            .tables
            .iter_mut()
            .find(|t| &t.name == table_name)
            .ok_or_else(|| ApplyError::TableNotFound {
                schema: schema_str,
                name: table_name.as_ref().to_string(),
            })?;
        table.columns.push(column);
        Ok(())
    }

    fn drop_column(
        &mut self,
        table_name: &TableName,
        column_name: &ColumnName,
    ) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let table_name_str = table_name.as_ref().to_string();
        let table = self
            .tables
            .iter_mut()
            .find(|t| &t.name == table_name)
            .ok_or_else(|| ApplyError::TableNotFound {
                schema: schema_str.clone(),
                name: table_name_str.clone(),
            })?;
        let pos = table
            .columns
            .iter()
            .position(|c| &c.name == column_name)
            .ok_or_else(|| ApplyError::ColumnNotFound {
                schema: schema_str,
                table: table_name_str,
                column: column_name.as_ref().to_string(),
            })?;
        table.columns.remove(pos);
        Ok(())
    }

    fn rename_column(
        &mut self,
        table_name: &TableName,
        from: &ColumnName,
        to: &ColumnName,
    ) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let table_name_str = table_name.as_ref().to_string();
        let table = self
            .tables
            .iter_mut()
            .find(|t| &t.name == table_name)
            .ok_or_else(|| ApplyError::TableNotFound {
                schema: schema_str.clone(),
                name: table_name_str.clone(),
            })?;
        let column = table
            .columns
            .iter_mut()
            .find(|c| &c.name == from)
            .ok_or_else(|| ApplyError::ColumnNotFound {
                schema: schema_str,
                table: table_name_str,
                column: from.as_ref().to_string(),
            })?;
        column.name = to.clone();
        Ok(())
    }

    fn alter_column(
        &mut self,
        table_name: &TableName,
        column_name: &ColumnName,
        changes: &ColumnChanges,
    ) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let table_name_str = table_name.as_ref().to_string();
        let table = self
            .tables
            .iter_mut()
            .find(|t| &t.name == table_name)
            .ok_or_else(|| ApplyError::TableNotFound {
                schema: schema_str.clone(),
                name: table_name_str.clone(),
            })?;
        let column = table
            .columns
            .iter_mut()
            .find(|c| &c.name == column_name)
            .ok_or_else(|| ApplyError::ColumnNotFound {
                schema: schema_str,
                table: table_name_str,
                column: column_name.as_ref().to_string(),
            })?;

        if let Some(ref set_type) = changes.set_type {
            column.type_info = set_type.type_info.clone();
        }
        if let Some(set_not_null) = changes.set_not_null {
            column.is_nullable = !set_not_null;
        }
        if let Some(ref set_default) = changes.set_default {
            match set_default {
                DefaultChange::Set(expr) => column.default = Some(expr.clone()),
                DefaultChange::Drop => column.default = None,
            }
        }
        if let Some(ref set_identity) = changes.set_identity {
            match set_identity {
                IdentityChange::Add(kind) => column.identity = Some(*kind),
                IdentityChange::Drop => column.identity = None,
            }
        }
        if let Some(ref set_generated) = changes.set_generated {
            match set_generated {
                GeneratedChange::Set(generated) => column.generated = Some(generated.clone()),
                GeneratedChange::Drop => column.generated = None,
            }
        }

        Ok(())
    }

    // =========================================================================
    // Constraint Operations
    // =========================================================================

    fn add_constraint(
        &mut self,
        table_name: &TableName,
        constraint: Constraint,
    ) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let table = self
            .tables
            .iter_mut()
            .find(|t| &t.name == table_name)
            .ok_or_else(|| ApplyError::TableNotFound {
                schema: schema_str,
                name: table_name.as_ref().to_string(),
            })?;
        table.constraints.push(constraint);
        Ok(())
    }

    fn drop_constraint(
        &mut self,
        table_name: &TableName,
        constraint_name: &ConstraintName,
    ) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let table_name_str = table_name.as_ref().to_string();
        let table = self
            .tables
            .iter_mut()
            .find(|t| &t.name == table_name)
            .ok_or_else(|| ApplyError::TableNotFound {
                schema: schema_str.clone(),
                name: table_name_str.clone(),
            })?;
        let pos = table
            .constraints
            .iter()
            .position(|c| &c.name == constraint_name)
            .ok_or_else(|| ApplyError::ConstraintNotFound {
                schema: schema_str,
                table: table_name_str,
                constraint: constraint_name.as_ref().to_string(),
            })?;
        table.constraints.remove(pos);
        Ok(())
    }

    fn rename_constraint(
        &mut self,
        table_name: &TableName,
        from: &ConstraintName,
        to: &ConstraintName,
    ) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let table_name_str = table_name.as_ref().to_string();
        let table = self
            .tables
            .iter_mut()
            .find(|t| &t.name == table_name)
            .ok_or_else(|| ApplyError::TableNotFound {
                schema: schema_str.clone(),
                name: table_name_str.clone(),
            })?;
        let constraint = table
            .constraints
            .iter_mut()
            .find(|c| &c.name == from)
            .ok_or_else(|| ApplyError::ConstraintNotFound {
                schema: schema_str,
                table: table_name_str,
                constraint: from.as_ref().to_string(),
            })?;
        constraint.name = to.clone();
        Ok(())
    }

    // =========================================================================
    // Index Operations
    // =========================================================================

    fn create_index(&mut self, table_name: &TableName, index: Index) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let table = self
            .tables
            .iter_mut()
            .find(|t| &t.name == table_name)
            .ok_or_else(|| ApplyError::TableNotFound {
                schema: schema_str,
                name: table_name.as_ref().to_string(),
            })?;
        table.indexes.push(index);
        Ok(())
    }

    fn drop_index(&mut self, index_name: &IndexName) -> Result<(), ApplyError> {
        // Indexes can be on any table, so we need to search all tables
        for table in &mut self.tables {
            if let Some(pos) = table.indexes.iter().position(|i| &i.name == index_name) {
                table.indexes.remove(pos);
                return Ok(());
            }
        }
        Err(ApplyError::IndexNotFound {
            schema: self.schema_str(),
            name: index_name.as_ref().to_string(),
        })
    }

    fn rename_index(&mut self, from: &IndexName, to: &IndexName) -> Result<(), ApplyError> {
        // Indexes can be on any table, so we need to search all tables
        for table in &mut self.tables {
            if let Some(index) = table.indexes.iter_mut().find(|i| &i.name == from) {
                index.name = to.clone();
                return Ok(());
            }
        }
        Err(ApplyError::IndexNotFound {
            schema: self.schema_str(),
            name: from.as_ref().to_string(),
        })
    }

    // =========================================================================
    // View Operations
    // =========================================================================

    fn create_view(&mut self, view: View) -> Result<(), ApplyError> {
        if self.views.iter().any(|v| v.name == view.name) {
            return Err(ApplyError::AlreadyExists {
                schema: self.schema_str(),
                name: view.name.as_ref().to_string(),
                kind: "view",
            });
        }
        self.views.push(view);
        Ok(())
    }

    fn drop_view(&mut self, name: &TableName) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let pos = self
            .views
            .iter()
            .position(|v| &v.name == name)
            .ok_or_else(|| ApplyError::ViewNotFound {
                schema: schema_str,
                name: name.as_ref().to_string(),
            })?;
        self.views.remove(pos);
        Ok(())
    }

    fn rename_view(&mut self, from: &TableName, to: &TableName) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let view = self
            .views
            .iter_mut()
            .find(|v| &v.name == from)
            .ok_or_else(|| ApplyError::ViewNotFound {
                schema: schema_str,
                name: from.as_ref().to_string(),
            })?;
        view.name = to.clone();
        Ok(())
    }

    fn replace_view(&mut self, new_view: View) -> Result<(), ApplyError> {
        let schema_str = self.schema_str();
        let view = self
            .views
            .iter_mut()
            .find(|v| v.name == new_view.name)
            .ok_or_else(|| ApplyError::ViewNotFound {
                schema: schema_str,
                name: new_view.name.as_ref().to_string(),
            })?;
        view.definition = new_view.definition;
        Ok(())
    }

    // =========================================================================
    // Comment Operations
    // =========================================================================

    fn set_comment(
        &mut self,
        target: &CommentTarget,
        comment: Option<String>,
    ) -> Result<(), ApplyError> {
        let comment = comment.map(crate::db::model::types::Comment::new);

        match target {
            CommentTarget::Schema(schema) => {
                self.verify_schema(schema)?;
                self.comment = comment;
            }
            CommentTarget::Table { schema, table } => {
                self.verify_schema(schema)?;
                let schema_str = self.schema_str();
                let t = self
                    .tables
                    .iter_mut()
                    .find(|tbl| &tbl.name == table)
                    .ok_or_else(|| ApplyError::TableNotFound {
                        schema: schema_str,
                        name: table.as_ref().to_string(),
                    })?;
                t.comment = comment;
            }
            CommentTarget::Column {
                schema,
                table,
                column,
            } => {
                self.verify_schema(schema)?;
                let schema_str = self.schema_str();
                let table_str = table.as_ref().to_string();
                let t = self
                    .tables
                    .iter_mut()
                    .find(|tbl| &tbl.name == table)
                    .ok_or_else(|| ApplyError::TableNotFound {
                        schema: schema_str.clone(),
                        name: table_str.clone(),
                    })?;
                let col = t
                    .columns
                    .iter_mut()
                    .find(|c| &c.name == column)
                    .ok_or_else(|| ApplyError::ColumnNotFound {
                        schema: schema_str,
                        table: table_str,
                        column: column.as_ref().to_string(),
                    })?;
                col.comment = comment;
            }
            CommentTarget::Index { schema, index } => {
                self.verify_schema(schema)?;
                // Search all tables for this index
                let mut found = false;
                for table in &mut self.tables {
                    if let Some(idx) = table.indexes.iter_mut().find(|i| &i.name == index) {
                        idx.comment = comment.clone();
                        found = true;
                        break;
                    }
                }
                if !found {
                    return Err(ApplyError::IndexNotFound {
                        schema: self.schema_str(),
                        name: index.as_ref().to_string(),
                    });
                }
            }
            CommentTarget::Constraint {
                schema,
                table,
                constraint,
            } => {
                self.verify_schema(schema)?;
                let schema_str = self.schema_str();
                let table_str = table.as_ref().to_string();
                let t = self
                    .tables
                    .iter_mut()
                    .find(|tbl| &tbl.name == table)
                    .ok_or_else(|| ApplyError::TableNotFound {
                        schema: schema_str.clone(),
                        name: table_str.clone(),
                    })?;
                let con = t
                    .constraints
                    .iter_mut()
                    .find(|c| &c.name == constraint)
                    .ok_or_else(|| ApplyError::ConstraintNotFound {
                        schema: schema_str,
                        table: table_str,
                        constraint: constraint.as_ref().to_string(),
                    })?;
                con.comment = comment;
            }
            CommentTarget::Sequence { schema, sequence } => {
                self.verify_schema(schema)?;
                let schema_str = self.schema_str();
                let seq = self
                    .sequences
                    .iter_mut()
                    .find(|s| &s.name == sequence)
                    .ok_or_else(|| ApplyError::SequenceNotFound {
                        schema: schema_str,
                        name: sequence.as_ref().to_string(),
                    })?;
                seq.comment = comment;
            }
            CommentTarget::Type { schema, type_name } => {
                self.verify_schema(schema)?;
                let schema_str = self.schema_str();
                let enum_type = self
                    .enums
                    .iter_mut()
                    .find(|e| &e.name == type_name)
                    .ok_or_else(|| ApplyError::EnumNotFound {
                        schema: schema_str,
                        name: type_name.as_ref().to_string(),
                    })?;
                enum_type.comment = comment;
            }
            CommentTarget::View { schema, view } => {
                self.verify_schema(schema)?;
                let schema_str = self.schema_str();
                let v = self
                    .views
                    .iter_mut()
                    .find(|vw| &vw.name == view)
                    .ok_or_else(|| ApplyError::ViewNotFound {
                        schema: schema_str,
                        name: view.as_ref().to_string(),
                    })?;
                v.comment = comment;
            }
        }

        Ok(())
    }
}

/// Generates a new unique OID for synthetic objects.
///
/// This is used when creating objects programmatically (not from a database)
/// where we need placeholder OIDs. The OIDs are sequential starting from a
/// high value to avoid collision with real OIDs.
#[derive(Debug, Default)]
pub struct OidGenerator {
    next: u32,
}

impl OidGenerator {
    /// Creates a new OID generator.
    ///
    /// Starts from a high value (1_000_000) to avoid collision with real
    /// PostgreSQL OIDs, which typically start low.
    #[must_use]
    pub fn new() -> Self {
        Self { next: 1_000_000 }
    }

    /// Generates the next OID.
    pub fn generate(&mut self) -> Oid {
        let oid = Oid::new(self.next);
        self.next += 1;
        oid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::model::TableKind;
    use crate::db::model::types::{SqlExpr, TypeInfo};

    fn make_type_info(name: &str) -> TypeInfo {
        TypeInfo {
            name: TypeName::try_new(name.to_string()).unwrap(),
            schema: SchemaName::try_new("pg_catalog".to_string()).unwrap(),
            formatted: name.to_string(),
            is_array: false,
        }
    }

    fn make_table(name: &str) -> Table {
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

    fn make_enum(name: &str, values: Vec<&str>) -> EnumType {
        EnumType {
            oid: Oid::new(1),
            name: TypeName::try_new(name.to_string()).unwrap(),
            values: values.into_iter().map(String::from).collect(),
            comment: None,
        }
    }

    fn make_sequence(name: &str) -> Sequence {
        Sequence {
            oid: Oid::new(1),
            name: SequenceName::try_new(name.to_string()).unwrap(),
            data_type: make_type_info("int8"),
            start_value: 1,
            increment: 1,
            min_value: 1,
            max_value: i64::MAX,
            cache_size: 1,
            is_cyclic: false,
            comment: None,
        }
    }

    fn make_view(name: &str, definition: &str) -> View {
        View {
            oid: Oid::new(1),
            name: TableName::try_new(name.to_string()).unwrap(),
            definition: SqlExpr::new(definition.to_string()),
            is_materialized: false,
            comment: None,
        }
    }

    mod enum_operations {
        use super::*;

        #[test]
        fn create_enum() {
            let ns = Namespace::empty("public");
            let enum_type = make_enum("status", vec!["pending", "active"]);

            let ops = vec![Operation::CreateEnum {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                enum_type: enum_type.clone(),
            }];

            let result = ns.apply(&ops).unwrap();
            assert_eq!(result.enums.len(), 1);
            assert_eq!(result.enums[0].name.as_ref(), "status");
            assert_eq!(result.enums[0].values, vec!["pending", "active"]);
        }

        #[test]
        fn drop_enum() {
            let mut ns = Namespace::empty("public");
            ns.enums
                .push(make_enum("status", vec!["pending", "active"]));

            let ops = vec![Operation::DropEnum {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                name: TypeName::try_new("status".to_string()).unwrap(),
            }];

            let result = ns.apply(&ops).unwrap();
            assert!(result.enums.is_empty());
        }

        #[test]
        fn rename_enum() {
            let mut ns = Namespace::empty("public");
            ns.enums
                .push(make_enum("status", vec!["pending", "active"]));

            let ops = vec![Operation::RenameEnum {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                from: TypeName::try_new("status".to_string()).unwrap(),
                to: TypeName::try_new("order_status".to_string()).unwrap(),
            }];

            let result = ns.apply(&ops).unwrap();
            assert_eq!(result.enums[0].name.as_ref(), "order_status");
        }

        #[test]
        fn add_enum_value_at_end() {
            let mut ns = Namespace::empty("public");
            ns.enums
                .push(make_enum("status", vec!["pending", "active"]));

            let ops = vec![Operation::AddEnumValue {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                enum_name: TypeName::try_new("status".to_string()).unwrap(),
                value: "completed".to_string(),
                position: EnumValuePosition::End,
            }];

            let result = ns.apply(&ops).unwrap();
            assert_eq!(
                result.enums[0].values,
                vec!["pending", "active", "completed"]
            );
        }

        #[test]
        fn add_enum_value_before() {
            let mut ns = Namespace::empty("public");
            ns.enums
                .push(make_enum("status", vec!["pending", "active"]));

            let ops = vec![Operation::AddEnumValue {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                enum_name: TypeName::try_new("status".to_string()).unwrap(),
                value: "reviewing".to_string(),
                position: EnumValuePosition::Before("active".to_string()),
            }];

            let result = ns.apply(&ops).unwrap();
            assert_eq!(
                result.enums[0].values,
                vec!["pending", "reviewing", "active"]
            );
        }

        #[test]
        fn add_enum_value_after() {
            let mut ns = Namespace::empty("public");
            ns.enums
                .push(make_enum("status", vec!["pending", "active"]));

            let ops = vec![Operation::AddEnumValue {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                enum_name: TypeName::try_new("status".to_string()).unwrap(),
                value: "reviewing".to_string(),
                position: EnumValuePosition::After("pending".to_string()),
            }];

            let result = ns.apply(&ops).unwrap();
            assert_eq!(
                result.enums[0].values,
                vec!["pending", "reviewing", "active"]
            );
        }
    }

    mod table_operations {
        use super::*;

        #[test]
        fn create_table() {
            let ns = Namespace::empty("public");
            let table = make_table("users");

            let ops = vec![Operation::CreateTable {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                table,
            }];

            let result = ns.apply(&ops).unwrap();
            assert_eq!(result.tables.len(), 1);
            assert_eq!(result.tables[0].name.as_ref(), "users");
        }

        #[test]
        fn drop_table() {
            let mut ns = Namespace::empty("public");
            ns.tables.push(make_table("users"));

            let ops = vec![Operation::DropTable {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                name: TableName::try_new("users".to_string()).unwrap(),
            }];

            let result = ns.apply(&ops).unwrap();
            assert!(result.tables.is_empty());
        }

        #[test]
        fn rename_table() {
            let mut ns = Namespace::empty("public");
            ns.tables.push(make_table("users"));

            let ops = vec![Operation::RenameTable {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                from: TableName::try_new("users".to_string()).unwrap(),
                to: TableName::try_new("accounts".to_string()).unwrap(),
            }];

            let result = ns.apply(&ops).unwrap();
            assert_eq!(result.tables[0].name.as_ref(), "accounts");
        }
    }

    mod sequence_operations {
        use super::*;

        #[test]
        fn create_sequence() {
            let ns = Namespace::empty("public");
            let sequence = make_sequence("users_id_seq");

            let ops = vec![Operation::CreateSequence {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                sequence,
            }];

            let result = ns.apply(&ops).unwrap();
            assert_eq!(result.sequences.len(), 1);
            assert_eq!(result.sequences[0].name.as_ref(), "users_id_seq");
        }

        #[test]
        fn alter_sequence() {
            let mut ns = Namespace::empty("public");
            ns.sequences.push(make_sequence("users_id_seq"));

            let ops = vec![Operation::AlterSequence {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                name: SequenceName::try_new("users_id_seq".to_string()).unwrap(),
                changes: SequenceChanges {
                    increment: Some(10),
                    cache_size: Some(100),
                    ..Default::default()
                },
            }];

            let result = ns.apply(&ops).unwrap();
            assert_eq!(result.sequences[0].increment, 10);
            assert_eq!(result.sequences[0].cache_size, 100);
        }
    }

    mod view_operations {
        use super::*;

        #[test]
        fn create_view() {
            let ns = Namespace::empty("public");
            let view = make_view("active_users", "SELECT * FROM users WHERE active");

            let ops = vec![Operation::CreateView {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                view,
            }];

            let result = ns.apply(&ops).unwrap();
            assert_eq!(result.views.len(), 1);
            assert_eq!(result.views[0].name.as_ref(), "active_users");
        }

        #[test]
        fn replace_view() {
            let mut ns = Namespace::empty("public");
            ns.views.push(make_view(
                "active_users",
                "SELECT * FROM users WHERE active",
            ));

            let new_view = make_view("active_users", "SELECT id, name FROM users WHERE active");

            let ops = vec![Operation::ReplaceView {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                view: new_view,
            }];

            let result = ns.apply(&ops).unwrap();
            assert_eq!(
                result.views[0].definition.as_ref(),
                "SELECT id, name FROM users WHERE active"
            );
        }
    }

    mod error_cases {
        use super::*;

        #[test]
        fn schema_mismatch() {
            let ns = Namespace::empty("public");
            let table = make_table("users");

            let ops = vec![Operation::CreateTable {
                schema: SchemaName::try_new("other_schema".to_string()).unwrap(),
                table,
            }];

            let result = ns.apply(&ops);
            assert!(matches!(result, Err(ApplyError::SchemaMismatch { .. })));
        }

        #[test]
        fn table_not_found() {
            let ns = Namespace::empty("public");

            let ops = vec![Operation::DropTable {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                name: TableName::try_new("nonexistent".to_string()).unwrap(),
            }];

            let result = ns.apply(&ops);
            assert!(matches!(result, Err(ApplyError::TableNotFound { .. })));
        }

        #[test]
        fn table_already_exists() {
            let mut ns = Namespace::empty("public");
            ns.tables.push(make_table("users"));

            let ops = vec![Operation::CreateTable {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                table: make_table("users"),
            }];

            let result = ns.apply(&ops);
            assert!(matches!(result, Err(ApplyError::AlreadyExists { .. })));
        }

        #[test]
        fn enum_value_not_found() {
            let mut ns = Namespace::empty("public");
            ns.enums
                .push(make_enum("status", vec!["pending", "active"]));

            let ops = vec![Operation::AddEnumValue {
                schema: SchemaName::try_new("public".to_string()).unwrap(),
                enum_name: TypeName::try_new("status".to_string()).unwrap(),
                value: "new_value".to_string(),
                position: EnumValuePosition::Before("nonexistent".to_string()),
            }];

            let result = ns.apply(&ops);
            assert!(matches!(result, Err(ApplyError::EnumValueNotFound { .. })));
        }
    }

    mod oid_generator {
        use super::*;

        #[test]
        fn generates_sequential_oids() {
            let mut generator = OidGenerator::new();
            let oid1 = generator.generate();
            let oid2 = generator.generate();
            let oid3 = generator.generate();

            assert_eq!(*oid1.as_ref(), 1_000_000);
            assert_eq!(*oid2.as_ref(), 1_000_001);
            assert_eq!(*oid3.as_ref(), 1_000_002);
        }
    }
}
