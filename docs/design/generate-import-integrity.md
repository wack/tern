# Design Document: Integrity Verification for Generate and Import Commands

## Overview

This document describes integrity verification requirements for the `tern generate` and `tern import` commands. These commands create new migrations by comparing a "source" state (derived from existing migrations) against a "target" state (from schema.sql or a live database). Ensuring the source state is accurate is critical for generating correct migrations.

This document complements the main [Migration Integrity Verification](./migration-integrity-verification.md) design, which covers verification during `migrate up` and `migrate down` commands.

## Command Comparison

| Command | Source State | Target State | Database Connection |
|---------|--------------|--------------|---------------------|
| `tern generate` | Local migrations | Local `schema.sql` via PGLite | No |
| `tern import` | Local migrations | Live database | Yes |

Both commands share the same source: the schema state reconstructed by replaying local migration operations. If the local migration files are corrupted or inconsistent, the source state will be wrong, and the generated migration will be incorrect.

## Why Verification Matters

### The Source State Problem

When generating a migration, Tern computes:

```
diff(source_state, target_state) → migration_operations
```

If `source_state` is wrong, the diff produces incorrect operations:

**Scenario: Broken migration chain**
```
Migration 1: CREATE TABLE users (id INT)
Migration 2: ADD COLUMN users.name       [parent_hash: <hash of state after migration 1>]
Migration 3: ADD COLUMN users.email      [parent_hash: WRONG - file was corrupted]

Tern reconstructs state from migrations...
→ source_state is indeterminate or wrong
→ generate produces incorrect diff
```

**Scenario: Modified migration file (import)**
```
Migration 3 applied to database with: ADD COLUMN status VARCHAR(50)
Developer modifies migration 3 locally: ADD COLUMN status user_status_enum
Developer runs `tern import`...

Local source_state: has ENUM column
Database target_state: has VARCHAR column
Diff: ALTER COLUMN status TYPE user_status_enum  ← Wrong! Should be no diff for this column
```

### Different Verification Needs

The two commands have different verification requirements because they serve different purposes:

| Aspect | `generate` | `import` |
|--------|------------|----------|
| Purpose | Capture local schema.sql changes | Capture live database drift |
| Has DB connection | No | Yes |
| Can verify against `tern.migrations` table | No | Yes |
| Schema checksum mismatch expected? | No | **Yes** (that's the point) |

## Proposed Verification

### For `tern generate`

Since there's no database connection, we can only verify local consistency.

**Verification: Migration chain integrity**

Before generating, verify that the local migration chain is self-consistent:
- Each migration's `parent_state_hash` matches the previous migration's `resulting_state_hash`
- No gaps in the chain
- Index file matches actual migration files

```
generate (proposed flow):

  1. Load local migration index
  2. VERIFY: Migration chain integrity
     → For each migration[i], check:
        migration[i].parent_state_hash == migration[i-1].resulting_state_hash
     → If mismatch: Error "Migration chain is broken"
  3. Reconstruct source_state from migrations
  4. Load target_state from schema.sql (via PGLite)
  5. Compute diff and generate migration
  6. Record new migration
```

**Error message for broken chain:**
```
Error: Migration chain integrity check failed

Migration 00004 (d4e5f6a7) has an invalid parent hash.

  Expected parent: b2c3d4e5... (from migration 00003)
  Actual parent:   9a8b7c6d... (in migration 00004)

This indicates the migration file was modified or corrupted.

To resolve:
  Option 1: Restore migration 00004 from version control
  Option 2: Run 'tern verify chain' for detailed diagnostics
  Option 3: Use --force to skip verification (dangerous)
```

### For `tern import`

Since `import` connects to a database, we can verify against the `tern.migrations` table.

**Verification 1: Migration chain integrity (local)**

Same as `generate` — verify local files are self-consistent.

**Verification 2: Migration hash verification (against database)**

Before importing, verify that local migration files match what was applied to this database:
- For each migration in `tern.migrations`, compute BLAKE3 hash of local `up_operations`
- Compare against stored `migration_hash`
- If mismatch, the local "source" state doesn't match what the database was built from

**NOT verified: Schema checksum**

The schema checksum should NOT be verified during `import` because:
- The purpose of `import` is to capture schema drift
- A mismatching schema is expected — that's why you're running `import`
- Requiring schema match would make `import` useless

```
import (proposed flow):

  1. Load local migration index
  2. VERIFY: Migration chain integrity (local)
  3. Connect to database
  4. Load tern.migrations table
  5. VERIFY: Migration hash integrity
     → For each recorded migration, compare local hash vs stored hash
     → If mismatch: Error "Local migrations don't match applied migrations"
  6. Reconstruct source_state from local migrations
  7. Load target_state from live database
  8. Compute diff and generate migration
  9. Record new migration (locally and in tern.migrations)
```

**Error message for migration hash mismatch:**
```
Error: Local migration files don't match applied migrations

Migration 00003 (a1b2c3d4) has been modified since it was applied to this database.

  Applied hash:  5e6f7a8b9c0d...
  Local hash:    1a2b3c4d5e6f...

The local migration file differs from what was applied to the database.
Running 'import' would generate an incorrect migration.

To resolve:
  Option 1: Restore migration 00003 from version control
  Option 2: Pull the correct migration files from the team
  Option 3: Use --force to skip verification (dangerous)

Warning: Proceeding with mismatched migrations can cause schema
inconsistencies between environments.
```

## Verification Summary

| Check | `generate` | `import` | Rationale |
|-------|------------|----------|-----------|
| Migration chain integrity (local) | **Yes** | **Yes** | Ensures source state is derived from valid chain |
| Migration hash vs `tern.migrations` | N/A | **Yes** | Ensures local files match what was applied |
| Schema checksum vs `tern.migrations` | N/A | **No** | Drift is the purpose of import |

## Implementation Changes

### 1. Add Chain Verification to Generate

**File:** `src/cli/generate/mod.rs`

```rust
impl Generate {
    pub async fn dispatch(self) -> miette::Result<()> {
        let backend = load_backend(self.path.as_deref());
        ensure_backend_initialized(&backend).await?;

        // NEW: Verify migration chain integrity
        if !self.force {
            backend.verify_chain().await
                .map_err(|e| miette::miette!("Migration chain verification failed: {}", e))?;
        }

        // Load the source state (current state from migrations)
        let source = backend.get_current_state().await.into_diagnostic()?;
        // ... rest of existing implementation
    }
}
```

### 2. Add `--force` Flag to Generate

**File:** `src/cli/generate/mod.rs`

The `--force` flag already exists for skipping destructive change confirmation. Extend its meaning to also skip chain verification:

```rust
#[derive(Debug, Clone, Args)]
pub struct Generate {
    // ... existing fields ...

    /// Skip confirmation prompt for destructive changes AND skip integrity verification
    #[arg(long)]
    pub force: bool,
}
```

Update the help text to clarify:
```
--force    Skip confirmation prompts and integrity verification (dangerous)
```

### 3. Add Migration Hash Verification to Import

**File:** `src/cli/import/mod.rs`

```rust
use crate::db::execution::tracker::MigrationTracker;

impl Import {
    pub async fn dispatch(self) -> miette::Result<()> {
        let backend = load_backend(self.path.as_deref());
        ensure_backend_initialized(&backend).await?;

        // NEW: Verify local chain integrity
        if !self.force {
            backend.verify_chain().await
                .map_err(|e| miette::miette!("Migration chain verification failed: {}", e))?;
        }

        println!("Connecting to database...");
        let client = db::connect(&db_url).await.into_diagnostic()?;

        // NEW: Verify migration hashes against database
        if !self.force {
            let tracker = MigrationTracker::new(&client, &self.schema);
            self.verify_migration_hashes(&backend, &tracker).await?;
        }

        // ... rest of existing implementation
    }

    async fn verify_migration_hashes(
        &self,
        backend: &LocalFileBackend,
        tracker: &MigrationTracker<'_>,
    ) -> miette::Result<()> {
        // Load all local migrations with their hashes
        let local_migrations = backend.get_all_migrations().await.into_diagnostic()?;
        let local_hashes: Vec<(MigrationId, String)> = local_migrations
            .iter()
            .map(|m| (m.id, compute_migration_hash(m)))
            .collect();

        // Verify against database
        let diverged = tracker.verify_history(&local_hashes).await
            .map_err(|e| miette::miette!("Failed to verify migration history: {}", e))?;

        if let Some((id, expected, actual)) = diverged.first() {
            return Err(miette::miette!(
                "Local migration files don't match applied migrations\n\n\
                 Migration {} has been modified since it was applied.\n\n\
                 Applied hash:  {}\n\
                 Local hash:    {}\n\n\
                 Restore the original migration from version control or use --force.",
                &id[..16.min(id.len())],
                &expected[..16.min(expected.len())],
                &actual[..16.min(actual.len())]
            ));
        }

        Ok(())
    }
}
```

### 4. Add `--force` Flag to Import

**File:** `src/cli/import/mod.rs`

```rust
#[derive(Debug, Clone, Args)]
pub struct Import {
    // ... existing fields ...

    /// Skip integrity verification (dangerous)
    #[arg(long)]
    pub force: bool,
}
```

### 5. Expose `compute_migration_hash` Function

The `compute_migration_hash` function is currently private in `executor.rs`. Move it to a shared location:

**File:** `src/db/execution/mod.rs`

```rust
mod hash;
pub use hash::compute_migration_hash;
```

**File:** `src/db/execution/hash.rs`

```rust
use crate::db::state::Migration;

/// Computes the BLAKE3 hash of a migration's up_operations.
///
/// This hash is used to detect if a migration has been modified since it was
/// applied. Only the up_operations array is hashed, not the description or
/// timestamps, allowing descriptions to be updated without triggering
/// divergence errors.
pub fn compute_migration_hash(migration: &Migration) -> String {
    let mut hasher = blake3::Hasher::new();
    let ops_json = serde_json::to_vec(&migration.up_operations)
        .expect("operations should be serializable");
    hasher.update(&ops_json);
    hasher.finalize().to_hex().to_string()
}
```

## Handling Edge Cases

### Fresh Database (No `tern.migrations` Table)

When `import` connects to a database without a `tern.migrations` table:
- Skip migration hash verification (nothing to verify against)
- This is the baseline import scenario

### Partial Migration History

If the database has fewer migrations than local:
- Only verify the migrations that exist in both
- The import will capture changes from where the DB left off

### More Migrations in Database Than Local

If the database has migrations not present locally:
- This is a sync error — local is behind
- Error with suggestion to pull latest migrations

```
Error: Database has migrations not present locally

The database has 10 migrations applied, but only 8 are present locally.
Your local migration history is behind the database.

To resolve:
  Pull the latest migrations from your team's repository.
```

## Testing Strategy

### Unit Tests

1. `verify_chain` catches broken parent hash chain
2. `verify_migration_hashes` catches modified local files
3. `--force` bypasses both verifications
4. Fresh database (no tracking table) doesn't error

### Integration Tests

1. Generate with valid chain succeeds
2. Generate with broken chain fails (without --force)
3. Generate with broken chain and --force succeeds
4. Import with matching hashes succeeds
5. Import with mismatched hashes fails (without --force)
6. Import with mismatched hashes and --force succeeds
7. Import to fresh database (no tern.migrations) succeeds

## Summary

| Command | Chain Verification | Hash Verification | Schema Verification |
|---------|-------------------|-------------------|---------------------|
| `generate` | ✅ Add | N/A | N/A |
| `import` | ✅ Add | ✅ Add | ❌ Skip (intentional) |

Both commands will gain a `--force` flag to bypass verification for emergency situations, with appropriate warnings about the risks.
