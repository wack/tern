//! Migration output tool implementations.

use crate::db::diff::breaking::{BreakingChange, BreakingChangeKind, analyze_breaking_changes};
use crate::db::diff::diff_namespaces;
use crate::db::migrate::{MigrationPlan, Operation, PostgresRenderer, RenderConfig, SqlOptions};
use crate::db::state::{LocalFileBackend, Migration, StateBackend, StateHash};
use crate::mcp::error::{McpError, McpResult};
use crate::mcp::server::{
    BreakingChangeInfo, CommitMigrationInput, CommitMigrationOutput, GetBreakingChangesOutput,
    GetMigrationPreviewInput, GetMigrationPreviewOutput, TernMcpService,
};

/// Helper to count creates/drops/alters in operations.
fn count_operation_types(ops: &[Operation]) -> (usize, usize, usize) {
    let mut creates = 0;
    let mut drops = 0;
    let mut alters = 0;

    for op in ops {
        match op {
            Operation::CreateTable { .. }
            | Operation::CreateEnum { .. }
            | Operation::CreateSequence { .. }
            | Operation::CreateIndex { .. } => creates += 1,
            Operation::DropTable { .. }
            | Operation::DropEnum { .. }
            | Operation::DropSequence { .. }
            | Operation::DropIndex { .. }
            | Operation::DropColumn { .. }
            | Operation::DropConstraint { .. } => drops += 1,
            _ => alters += 1,
        }
    }

    (creates, drops, alters)
}

/// Helper to get a string representation of breaking change kind.
fn breaking_change_kind_name(kind: &BreakingChangeKind) -> &'static str {
    match kind {
        // Table changes
        BreakingChangeKind::TableDropped { .. } => "table_dropped",
        BreakingChangeKind::TableRenamed { .. } => "table_renamed",
        // Column changes
        BreakingChangeKind::ColumnDropped { .. } => "column_dropped",
        BreakingChangeKind::ColumnRenamed { .. } => "column_renamed",
        BreakingChangeKind::ColumnTypeChanged { .. } => "column_type_changed",
        BreakingChangeKind::ColumnMadeNonNullable { .. } => "column_made_non_nullable",
        // Constraint changes
        BreakingChangeKind::PrimaryKeyAdded { .. } => "primary_key_added",
        BreakingChangeKind::UniqueConstraintAdded { .. } => "unique_constraint_added",
        BreakingChangeKind::CheckConstraintAdded { .. } => "check_constraint_added",
        BreakingChangeKind::ForeignKeyAdded { .. } => "foreign_key_added",
        BreakingChangeKind::ExclusionConstraintAdded { .. } => "exclusion_constraint_added",
        // Enum changes
        BreakingChangeKind::EnumValueRemoved { .. } => "enum_value_removed",
        BreakingChangeKind::EnumValuesReordered { .. } => "enum_values_reordered",
        // View changes
        BreakingChangeKind::ViewDropped { .. } => "view_dropped",
        BreakingChangeKind::ViewRenamed { .. } => "view_renamed",
        BreakingChangeKind::MaterializationChanged { .. } => "materialization_changed",
        // Sequence changes
        BreakingChangeKind::SequenceDropped { .. } => "sequence_dropped",
        BreakingChangeKind::SequenceRenamed { .. } => "sequence_renamed",
    }
}

/// Implements the get_migration_preview tool.
pub async fn get_migration_preview(
    service: &TernMcpService,
    input: GetMigrationPreviewInput,
) -> McpResult<GetMigrationPreviewOutput> {
    let session_guard = service.require_session().await?;
    let session = session_guard.as_ref().unwrap();

    // Get current namespace from PGLite
    let current_namespace = session.current_namespace().await?;

    // Diff against base namespace
    let diff = diff_namespaces(session.base_namespace(), &current_namespace);

    // Analyze breaking changes
    let analysis = analyze_breaking_changes(&diff);
    let has_breaking_changes = !analysis.is_safe();

    // Create migration plan
    let plan = MigrationPlan::from_diff(&diff);

    if plan.operations.is_empty() {
        return Err(McpError::NoChanges);
    }

    // Render to SQL
    let renderer = PostgresRenderer::new(RenderConfig::default());
    let script = plan.render(&renderer);

    // Build SQL with options
    let sql = script.to_sql_with_options(SqlOptions {
        include_transaction: input.include_transaction,
        include_comments: input.include_comments,
    });

    // Build summary
    let (creates, drops, alters) = count_operation_types(&plan.operations);

    let mut summary_parts = Vec::new();
    if creates > 0 {
        summary_parts.push(format!("{} create(s)", creates));
    }
    if alters > 0 {
        summary_parts.push(format!("{} alter(s)", alters));
    }
    if drops > 0 {
        summary_parts.push(format!("{} drop(s)", drops));
    }
    let summary = summary_parts.join(", ");

    Ok(GetMigrationPreviewOutput {
        sql,
        operation_count: plan.operations.len(),
        has_breaking_changes,
        summary,
    })
}

/// Implements the get_breaking_changes tool.
pub async fn get_breaking_changes(service: &TernMcpService) -> McpResult<GetBreakingChangesOutput> {
    let session_guard = service.require_session().await?;
    let session = session_guard.as_ref().unwrap();

    // Get current namespace from PGLite
    let current_namespace = session.current_namespace().await?;

    // Diff against base namespace
    let diff = diff_namespaces(session.base_namespace(), &current_namespace);

    // Analyze breaking changes
    let analysis = analyze_breaking_changes(&diff);

    let changes: Vec<BreakingChangeInfo> = analysis
        .iter()
        .map(|change| BreakingChangeInfo {
            r#type: breaking_change_kind_name(&change.kind).to_string(),
            description: change.description.clone(),
            mitigation: change.mitigation.as_str().to_string(),
        })
        .collect();

    Ok(GetBreakingChangesOutput {
        has_breaking_changes: !analysis.is_safe(),
        changes,
    })
}

/// Implements the commit_migration tool.
pub async fn commit_migration(
    service: &TernMcpService,
    input: CommitMigrationInput,
) -> McpResult<CommitMigrationOutput> {
    let session_guard = service.require_session().await?;
    let session = session_guard.as_ref().unwrap();

    // Get current namespace from PGLite
    let current_namespace = session.current_namespace().await?;

    // Diff against base namespace
    let diff = diff_namespaces(session.base_namespace(), &current_namespace);

    // Analyze breaking changes
    let analysis = analyze_breaking_changes(&diff);

    if !analysis.is_safe() && !input.force {
        let breaking_descriptions: Vec<String> =
            analysis.iter().map(|c| c.description.clone()).collect();
        return Err(McpError::BreakingChanges {
            changes: breaking_descriptions,
        });
    }

    // Create migration plan
    let plan = MigrationPlan::from_diff(&diff);

    if plan.operations.is_empty() {
        return Err(McpError::NoChanges);
    }

    // Get the state backend
    let backend = LocalFileBackend::new(session.working_dir());

    // Get parent state hash
    let parent_hash = backend.get_current_state_hash().await?;

    // Compute new state hash
    let new_hash = StateHash::from_namespace(&current_namespace);

    // Collect breaking changes for the migration
    let breaking_changes: Vec<BreakingChange> = analysis.iter().cloned().collect();

    // Create the migration
    let migration = Migration::new(
        &input.description,
        plan.operations.clone(),
        parent_hash,
        new_hash,
        breaking_changes,
    );

    let migration_id = migration.id.to_string();

    // Record the migration
    backend
        .record_migration(&migration, &current_namespace)
        .await?;

    // Generate SQL preview
    let renderer = PostgresRenderer::new(RenderConfig::default());
    let script = plan.render(&renderer);
    let sql_preview = script.to_sql();

    // Build migration path
    let migration_path = format!(
        ".tern/migrations/{}.json",
        &migration_id[..16.min(migration_id.len())]
    );

    Ok(CommitMigrationOutput {
        success: true,
        migration_id,
        migration_path,
        sql_preview,
        message: format!("Migration committed: '{}'", input.description),
    })
}
