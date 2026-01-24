# Design Document: Migration Integrity Verification

## Overview

This document describes the design for comprehensive integrity verification during migration execution. The goal is to ensure that both the database schema and migration history are in the expected state before applying or reverting migrations.

### Core Problem

When executing migrations, Tern must answer two critical questions:

1. **Is the database in the expected state?** Before applying migration N, the database should match the schema that resulted from applying migration N-1.

2. **Has the migration history been tampered with?** The migration files on disk should match what was originally applied to this database.

Without verifying both conditions, migrations can silently corrupt the database or produce inconsistent states across environments.

### Why This Matters

Consider these failure scenarios that proper integrity verification would prevent:

**Scenario 1: Manual database modification**
```
Developer A runs migrations 1-5 on staging.
DBA manually adds an index to staging for performance testing.
Developer B runs migration 6, which also tries to create that index.
Result: Migration 6 fails with "index already exists" error.
```

**Scenario 2: Modified migration file**
```
Developer A creates and applies migration 3 (adds column `status VARCHAR(50)`).
Developer A realizes the column should be an enum, modifies migration 3 on disk.
Developer B pulls the modified migration 3 and runs `migrate up`.
Result: Database has VARCHAR column, new environments get ENUM column. Schema drift.
```

**Scenario 3: Divergent environments**
```
Production has migrations 1-10 applied.
A deployment runs `migrate up` assuming it's at migration 8.
Migrations 9-10 run again, or fail, or produce duplicate data.
```

All these scenarios are preventable with proper integrity verification.

## Integrity Verification Design

### Two Types of Integrity Checks

Tern maintains two independent hashes for integrity verification:

| Hash Type | Algorithm | What It Verifies | Stored In |
|-----------|-----------|------------------|-----------|
| **Schema Checksum** | xxhash3 (64-bit) | The actual database schema structure | `tern.migrations.schema_hash` |
| **Migration Hash** | BLAKE3 (256-bit) | The content of migration operations | `tern.migrations.migration_hash` |

These serve complementary purposes:

**Schema Checksum (xxhash3)**
- Fast, non-cryptographic hash of the `Namespace` structure
- Detects when the database has been modified outside of Tern
- Answers: "Is the live database what we expect it to be?"

**Migration Hash (BLAKE3)**
- Cryptographic hash of `up_operations` array
- Detects when migration files have been modified after being applied
- Answers: "Are the migration files authentic?"

### Why Two Hashes?

A single hash cannot serve both purposes:

1. **Schema changes without migration changes**: Someone runs `ALTER TABLE` directly on the database. The migration files haven't changed, but the schema has. Only the schema checksum detects this.

2. **Migration changes without schema changes**: Someone modifies a migration file that was already applied. The live schema hasn't changed (it was applied with the old version), but future environments will get different schemas. Only the migration hash detects this.

3. **Both can be correct while state is wrong**: If we only checked one, we might miss the other class of problems.

### The `tern.migrations` Table

```sql
CREATE TABLE tern.migrations (
    id              TEXT PRIMARY KEY,      -- Migration ID (BLAKE3 of content)
    sequence        INTEGER NOT NULL UNIQUE, -- Sequential ordering
    description     TEXT NOT NULL,
    migration_hash  TEXT NOT NULL,         -- BLAKE3 of up_operations
    schema_hash     TEXT NOT NULL,         -- xxhash3 of resulting Namespace
    applied_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

After applying migration N:
- `migration_hash` = BLAKE3 hash of migration N's `up_operations`
- `schema_hash` = xxhash3 checksum of the database schema after migration N

### Verification Points

Integrity checks should occur at these points:

| Command | Before Execution | After Execution |
|---------|-----------------|-----------------|
| `migrate up` | Verify schema matches last `schema_hash`; Verify all migration hashes | Compute and store new `schema_hash` |
| `migrate down` | Verify schema matches current `schema_hash`; Verify current migration hash | Compute and store previous `schema_hash` |

## Current Implementation Status

### What's Implemented

**Migration hash verification during `migrate up`:**
- Location: `src/db/execution/executor.rs:82` calls `verify_history()`
- Location: `src/db/execution/tracker.rs:216-239` implements the check
- Compares BLAKE3 hash of local `up_operations` against stored `migration_hash`
- Raises `ExecutionError::HistoryDiverged` if mismatch

**Schema hash storage:**
- Location: `src/db/execution/executor.rs:223` computes xxhash3 after applying migration
- Location: `src/db/execution/executor.rs:229-236` stores in `tern.migrations.schema_hash`
- The hash is computed and stored but never verified

**`tern verify` command (manual, separate):**
- Location: `src/cli/verify/mod.rs:62-128`
- Compares local `state.json` checksum against live database
- Not integrated into `migrate up` or `migrate down`
- Uses `state.json` which is machine-local (not in VCS)

### What's Missing

| Gap | Impact | Severity |
|-----|--------|----------|
| No schema verification before `migrate up` | Manual DB changes go undetected until migration fails | High |
| No schema verification before `migrate down` | Reverting from unexpected state can corrupt DB | High |
| No migration hash verification during `migrate down` | Modified down_operations could revert incorrectly | Medium |
| `tern verify` uses local `state.json` instead of DB's `schema_hash` | Can't detect drift consistently across machines | Medium |

### Current Data Flow

```
migrate up (current):

  1. Load migrations from disk (.tern/migrations/*.json)
  2. Query tern.current → get current_id
  3. verify_history() → compare migration_hash values ✓
  4. Find pending migrations
  5. For each pending:
     a. Execute up_operations SQL
     b. Compute schema_hash ✓ (stored but never verified)
     c. Compute migration_hash ✓
     d. INSERT into tern.migrations
     e. UPDATE tern.current

  Missing: Step 3.5 - Verify live schema matches expected schema_hash
```

```
migrate down (current):

  1. Query tern.current → get current_id
  2. Load migration from disk by ID
  3. Execute down_operations SQL
  4. DELETE from tern.migrations
  5. UPDATE tern.current

  Missing: Step 2.5 - Verify migration_hash matches
  Missing: Step 2.5 - Verify live schema matches schema_hash
```

## Proposed Design

### Verification Before `migrate up`

Before applying any pending migrations, verify:

1. **Schema integrity**: The live database schema checksum must match the `schema_hash` stored for the current migration (the last one applied).

2. **Migration history integrity**: All migration files on disk must have `migration_hash` values matching what's stored in `tern.migrations`.

```
migrate up (proposed):

  1. Load migrations from disk
  2. Query tern.current → get current_id
  3. Query tern.migrations WHERE id = current_id → get expected schema_hash
  4. Compute live database schema checksum
  5. VERIFY: live checksum == expected schema_hash
     → If mismatch: Error "Schema drift detected"
  6. verify_history() → compare all migration_hash values
     → If mismatch: Error "Migration history diverged"
  7. Find pending migrations
  8. For each pending:
     a. Execute up_operations SQL
     b. Compute and store schema_hash
     c. Compute and store migration_hash
     d. Record in tern.migrations
```

### Verification Before `migrate down`

Before reverting the current migration, verify:

1. **Schema integrity**: The live database must match the `schema_hash` stored for the current migration.

2. **Migration integrity**: The migration file's `up_operations` hash must match the stored `migration_hash`. (This ensures we're reverting what was actually applied.)

```
migrate down (proposed):

  1. Query tern.current → get current_id
  2. Query tern.migrations WHERE id = current_id
     → get expected schema_hash, migration_hash
  3. Compute live database schema checksum
  4. VERIFY: live checksum == expected schema_hash
     → If mismatch: Error "Schema drift detected"
  5. Load migration from disk by ID
  6. Compute BLAKE3 hash of migration's up_operations
  7. VERIFY: computed hash == stored migration_hash
     → If mismatch: Error "Migration file has been modified"
  8. Execute down_operations SQL
  9. Compute new schema checksum (should match previous migration's schema_hash)
  10. DELETE from tern.migrations WHERE id = current_id
  11. UPDATE tern.current to previous migration
```

### Fresh Database Handling

When no migrations have been applied yet (`tern.current` is empty):

- **Schema verification**: Skip (no expected state)
- **Migration history verification**: Skip (nothing to compare)

The first migration is always applied without verification since there's no prior state.

### Error Messages

**Schema drift detected:**
```
Error: Schema drift detected

The live database schema does not match the expected state after migration 5.

  Expected checksum: a1b2c3d4e5f6
  Actual checksum:   f6e5d4c3b2a1

This indicates the database was modified outside of Tern migrations.

To investigate:
  tern verify --database-url <URL>

To resolve:
  Option 1: Revert manual changes to match expected state
  Option 2: Run `tern compile` to capture changes as a new migration
  Option 3: Use `--force` to skip verification (dangerous)
```

**Migration history diverged:**
```
Error: Migration history diverged

Migration 00003 (a1b2c3d4) has been modified since it was applied.

  Expected hash: 5e6f7a8b9c0d...
  Actual hash:   1a2b3c4d5e6f...

This migration was applied with different operations than what's on disk.

To resolve:
  Option 1: Restore the original migration file from version control
  Option 2: Use `--force` to skip verification (dangerous)

Warning: Running migrations with mismatched history can cause schema
inconsistencies between environments.
```

### Force Flag

For emergency situations, provide a `--force` flag that skips verification:

```bash
tern migrate up --force     # Skip all integrity checks
tern migrate down --force   # Skip all integrity checks
```

This should print a prominent warning:

```
Warning: Running with --force skips integrity verification.
This can result in schema corruption or data loss.
Only use this if you understand the risks.

Proceeding in 5 seconds... (Ctrl+C to cancel)
```

### Updating `tern verify`

The `tern verify` command should be updated to compare against the database's `schema_hash` rather than local `state.json`:

**Current behavior:**
- Compares `state.json` checksum vs live database
- `state.json` is machine-local, not shared

**Proposed behavior:**
- Compare live database checksum vs `tern.migrations.schema_hash` (for the current migration)
- This uses the shared source of truth in the database itself
- Optionally also compare against local `state.json` with a flag

```bash
# Compare live DB against DB's recorded schema_hash (new default)
tern verify --database-url <URL>

# Also compare against local state.json
tern verify --database-url <URL> --include-local-state
```

## Implementation Changes

### 1. Add `verify_schema_checksum` to `MigrationTracker`

**File:** `src/db/execution/tracker.rs`

```rust
/// Verifies that the live database schema matches the expected checksum.
///
/// Returns Ok(()) if checksums match, or an error describing the mismatch.
pub async fn verify_schema_checksum(
    &self,
    expected_checksum: &str,
) -> Result<(), ExecutionError> {
    // Load current namespace from database
    let catalog = PostgresCatalog::new(self.client);
    let namespace = load_namespace(&catalog, self.target_schema()).await
        .map_err(|e| ExecutionError::Query(e.to_string()))?;

    // Compute actual checksum
    let actual_checksum = compute_schema_checksum(&namespace);

    if actual_checksum != expected_checksum {
        return Err(ExecutionError::SchemaDrift {
            expected: expected_checksum.to_string(),
            actual: actual_checksum,
        });
    }

    Ok(())
}

/// Gets the schema_hash for a specific migration.
pub async fn get_schema_hash(&self, migration_id: &str) -> Result<Option<String>, ExecutionError> {
    let row = self
        .client
        .query_opt(
            "SELECT schema_hash FROM tern.migrations WHERE id = $1",
            &[&migration_id],
        )
        .await
        .map_err(|e| ExecutionError::Query(e.to_string()))?;

    Ok(row.map(|r| r.get(0)))
}
```

### 2. Add New Error Variants

**File:** `src/db/execution/error.rs`

```rust
#[derive(Debug, Error, Diagnostic)]
pub enum ExecutionError {
    // ... existing variants ...

    #[error("Schema drift detected")]
    #[diagnostic(
        code(tern::schema_drift),
        help("The database was modified outside of Tern. Run `tern verify` to investigate.")
    )]
    SchemaDrift {
        expected: String,
        actual: String,
    },

    #[error("Migration file has been modified since it was applied")]
    #[diagnostic(
        code(tern::migration_modified),
        help("Restore the original migration from version control or use --force.")
    )]
    MigrationModified {
        migration_id: String,
        expected_hash: String,
        actual_hash: String,
    },
}
```

### 3. Update `execute_pending` in `MigrationExecutor`

**File:** `src/db/execution/executor.rs`

```rust
pub async fn execute_pending(&self, force: bool) -> Result<ExecutionResult, ExecutionError> {
    // Ensure tracking infrastructure exists
    self.tracker.ensure_schema().await?;

    // Get current state
    let current_id = self.tracker.get_current_migration_id().await?;

    // NEW: Verify schema integrity before proceeding
    if !force {
        if let Some(ref id) = current_id {
            if let Some(expected_hash) = self.tracker.get_schema_hash(id).await? {
                self.tracker.verify_schema_checksum(&expected_hash).await?;
            }
        }
    }

    // Load all local migrations
    let all_migrations = self
        .backend
        .get_all_migrations()
        .await
        .map_err(|e| ExecutionError::InvalidState(e.to_string()))?;

    // Verify history integrity (existing)
    if !force {
        self.verify_history(&all_migrations).await?;
    }

    // ... rest of existing implementation ...
}
```

### 4. Update `migrate down` Command

**File:** `src/cli/migrate/down.rs`

Add schema and migration hash verification before executing down operations:

```rust
pub async fn execute_down(
    client: &Client,
    backend: &LocalFileBackend,
    target_schema: &str,
    force: bool,
) -> Result<DownResult, ExecutionError> {
    let tracker = MigrationTracker::new(client, target_schema);

    // Get current migration
    let current_id = tracker.get_current_migration_id().await?
        .ok_or(ExecutionError::NoMigrationsToRevert)?;

    let current_record = tracker.get_migration(&current_id).await?
        .ok_or(ExecutionError::MigrationNotFound(current_id.clone()))?;

    // NEW: Verify schema integrity
    if !force {
        tracker.verify_schema_checksum(&current_record.schema_hash).await?;
    }

    // Load migration from disk
    let migration = backend.get_migration(&MigrationId::from_hex(&current_id).unwrap())
        .await
        .map_err(|e| ExecutionError::InvalidState(e.to_string()))?;

    // NEW: Verify migration hash matches what was applied
    if !force {
        let local_hash = compute_migration_hash(&migration);
        if local_hash != current_record.migration_hash {
            return Err(ExecutionError::MigrationModified {
                migration_id: current_id,
                expected_hash: current_record.migration_hash,
                actual_hash: local_hash,
            });
        }
    }

    // Execute down operations
    // ... rest of implementation ...
}
```

### 5. Add `--force` Flag to CLI Commands

**File:** `src/cli/migrate/up.rs`

```rust
#[derive(Debug, Clone, Args)]
pub struct Up {
    // ... existing fields ...

    /// Skip integrity verification (dangerous)
    #[arg(long)]
    pub force: bool,
}
```

**File:** `src/cli/migrate/down.rs`

```rust
#[derive(Debug, Clone, Args)]
pub struct Down {
    // ... existing fields ...

    /// Skip integrity verification (dangerous)
    #[arg(long)]
    pub force: bool,
}
```

### 6. Update `tern verify` Command

**File:** `src/cli/verify/mod.rs`

```rust
impl Verify {
    pub async fn dispatch(self) -> miette::Result<()> {
        // Connect to database
        let client = db::connect(&self.database_url).await?;
        let tracker = MigrationTracker::new(&client, &self.schema);

        // Get current migration's expected schema_hash from DB
        let current_id = tracker.get_current_migration_id().await?;

        let expected_hash = if let Some(ref id) = current_id {
            tracker.get_schema_hash(id).await?
        } else {
            None
        };

        // Compute live schema checksum
        let catalog = PostgresCatalog::new(&client);
        let namespace = load_namespace(&catalog, &self.schema).await?;
        let actual_hash = compute_schema_checksum(&namespace);

        // Compare against DB's recorded hash (primary check)
        let db_verified = match expected_hash {
            Some(ref expected) => *expected == actual_hash,
            None => true, // No migrations applied yet
        };

        // Optionally compare against local state.json
        let local_verified = if self.include_local_state {
            let backend = load_backend(self.path.as_deref());
            let cached = backend.get_cached_state()?;
            cached.checksum() == actual_hash
        } else {
            true
        };

        // ... output results ...
    }
}
```

## Summary of Changes

| Component | Change | Priority |
|-----------|--------|----------|
| `MigrationTracker` | Add `verify_schema_checksum()` method | High |
| `MigrationTracker` | Add `get_schema_hash()` method | High |
| `ExecutionError` | Add `SchemaDrift` and `MigrationModified` variants | High |
| `MigrationExecutor::execute_pending` | Add schema verification before applying | High |
| `migrate down` command | Add schema and migration hash verification | High |
| CLI (`up`, `down`) | Add `--force` flag | Medium |
| `tern verify` | Compare against DB's `schema_hash` instead of `state.json` | Medium |
| Error messages | Add detailed diagnostics with resolution steps | Medium |

## Testing Strategy

### Unit Tests

1. `verify_schema_checksum` returns Ok when checksums match
2. `verify_schema_checksum` returns `SchemaDrift` error when checksums differ
3. `execute_pending` calls verification before applying (mock tracker)
4. `execute_pending` skips verification when `force=true`
5. Down migration fails when migration hash doesn't match
6. Down migration succeeds when `force=true` despite hash mismatch

### Integration Tests

1. Create migrations, apply them, manually alter DB, verify `migrate up` fails
2. Create migrations, apply them, modify migration file, verify `migrate up` fails
3. Create migrations, apply them, `migrate down` succeeds
4. Create migrations, apply them, modify migration file, verify `migrate down` fails
5. All above with `--force` flag should succeed (with warnings)

### Manual Testing Scenarios

1. Fresh database → apply migrations → verify checksums stored correctly
2. Apply migrations → run `psql ALTER TABLE` → attempt `migrate up` → expect failure
3. Apply migrations → edit migration JSON on disk → attempt `migrate up` → expect failure
4. Apply migrations → `tern verify` → expect success
5. Apply migrations → `psql ALTER TABLE` → `tern verify` → expect failure with diff

## Appendix: Hash Algorithm Choices

### Why xxhash3 for Schema Checksum?

- **Speed**: ~10x faster than BLAKE3 for this use case
- **Sufficient collision resistance**: 64-bit hash with birthday bound of ~2^32 is adequate for detecting accidental changes
- **Not security-critical**: We're detecting accidental drift, not malicious tampering
- **Consistency**: Already used elsewhere in Tern for schema checksums

### Why BLAKE3 for Migration Hash?

- **Cryptographic security**: Prevents intentional tampering with migration history
- **Consistency**: Migration IDs are already BLAKE3 hashes
- **Integrity chain**: Each migration's ID includes its parent's hash, creating a tamper-evident chain
