//! Session management tool implementations.

use crate::mcp::error::{McpError, McpResult};
use crate::mcp::server::{
    BaseStateInfo, SessionResetInput, SessionResetOutput, SessionStartInput, SessionStartOutput,
    SessionStatusOutput, TernMcpService,
};
use crate::mcp::session::Session;
use std::path::PathBuf;

/// Implements the session_start tool.
pub async fn session_start(
    service: &TernMcpService,
    input: SessionStartInput,
) -> McpResult<SessionStartOutput> {
    let mut session_guard = service.session.write().await;

    // Check if a session already exists
    if session_guard.is_some() {
        return Err(McpError::SessionExists);
    }

    // Determine working directory
    let working_dir = input
        .working_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| service.default_working_dir.clone());

    // Determine schema name
    let schema_name = input.schema_name.unwrap_or_else(|| "public".to_string());

    // Create the session
    let session = Session::new(working_dir, schema_name).await?;

    // Extract base state info
    let base_namespace = session.base_namespace();
    let tables: Vec<String> = base_namespace
        .tables
        .iter()
        .map(|t| t.name.as_ref().to_string())
        .collect();
    let enums: Vec<String> = base_namespace
        .enums
        .iter()
        .map(|e| e.name.as_ref().to_string())
        .collect();
    let sequences: Vec<String> = base_namespace
        .sequences
        .iter()
        .map(|s| s.name.as_ref().to_string())
        .collect();

    let table_count = tables.len();
    let enum_count = enums.len();
    let sequence_count = sequences.len();

    let session_id = session.id().to_string();

    // Store the session
    *session_guard = Some(session);

    Ok(SessionStartOutput {
        session_id,
        base_state: BaseStateInfo {
            tables,
            enums,
            sequences,
        },
        message: format!(
            "Session started. Base schema has {} table(s), {} enum(s), {} sequence(s).",
            table_count, enum_count, sequence_count
        ),
    })
}

/// Implements the session_status tool.
pub async fn session_status(service: &TernMcpService) -> McpResult<SessionStatusOutput> {
    let session_guard = service.require_session().await?;
    let session = session_guard.as_ref().unwrap();

    let pending_changes = session.pending_changes_summary();
    let has_breaking_changes =
        !pending_changes.tables_dropped.is_empty() || !pending_changes.columns_dropped.is_empty();

    Ok(SessionStatusOutput {
        session_id: session.id().to_string(),
        operations_applied: session.operation_count(),
        pending_changes,
        has_breaking_changes,
    })
}

/// Implements the session_reset tool.
pub async fn session_reset(
    service: &TernMcpService,
    input: SessionResetInput,
) -> McpResult<SessionResetOutput> {
    if !input.confirm {
        return Err(McpError::InvalidInput {
            message: "Must set confirm=true to reset session".to_string(),
        });
    }

    let mut session_guard = service.require_session_mut().await?;
    let session = session_guard.as_mut().unwrap();

    let discarded = session.reset().await?;

    Ok(SessionResetOutput {
        message: format!("Session reset. {} operation(s) discarded.", discarded),
        operations_discarded: discarded,
    })
}
