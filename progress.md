# Breaking Change Detection - Design Progress

## Current State: Implemented

We have a complete implementation of breaking change detection in `src/db/diff/breaking.rs`. This module analyzes schema diffs and classifies changes as either **safe** (can deploy directly) or **breaking** (requires a mitigation strategy).

### What Was Built

- `MitigationStrategy` enum with four strategies: `DualWrite`, `Backfill`, `Ratchet`, `Destructive`
- `BreakingChangeKind` enum with 17 specific change types, each mapped to a mitigation strategy
- `BreakingChange` struct with kind, mitigation strategy, and human-readable description
- `BreakingChangeAnalysis` aggregator with query methods (`is_safe()`, `by_mitigation()`, `count_by_mitigation()`)
- `analyze_breaking_changes()` function that walks a `NamespaceDiff`
- Type change classification (safe widening vs breaking narrowing)
- 94 passing tests (unit + integration)

### API Overview

```rust
use tern::db::diff::{diff_namespaces, NamespaceDiff};
use tern::db::diff::breaking::{analyze_breaking_changes, MitigationStrategy};

let diff = diff_namespaces(&source, &target);
let analysis = analyze_breaking_changes(&diff);

if analysis.is_safe() {
    println!("Migration is safe to apply directly");
} else {
    println!("Found {} breaking changes:", analysis.len());
    for change in analysis.iter() {
        println!("  [{}] {}", change.mitigation.as_str(), change.description);
    }

    // Query by mitigation strategy
    let ratchet_count = analysis.count_by_mitigation(MitigationStrategy::Ratchet);
    println!("Changes requiring NOT VALID pattern: {}", ratchet_count);
}
```

## Design Evolution

### Original Design (Rejected)

The original implementation used a `ChangeSeverity` enum with three levels:
- `NonBreaking` - Safe changes
- `Warning` - Might fail depending on data
- `Breaking` - Definitely problematic

The "Warning" category was flawed. **If a migration might fail, it IS breaking.** You cannot deploy it with confidence.

### Current Design: Mitigation Strategies

Rather than classifying by severity, we classify by **what kind of process is required to safely execute the change**:

| Strategy | Description | Examples |
|----------|-------------|----------|
| `DualWrite` | Requires parallel structures with synchronized writes | Rename column/table, change column type |
| `Backfill` | Requires populating data before completion | Add NOT NULL to existing column |
| `Ratchet` | Requires NOT VALID + backfill + VALIDATE pattern | Add UNIQUE/CHECK/FK/PK constraint |
| `Destructive` | Intentionally removes data/structure (irreversible) | Drop table/column, remove enum value |

### Why This Is Better

1. **Binary safety**: A change is either Safe or it requires mitigation - no ambiguous middle ground
2. **Actionable**: Each strategy implies a specific decomposition pattern
3. **Pattern-based**: Maps directly to known PostgreSQL migration patterns
4. **Time-aware**: Acknowledges that some changes fundamentally cannot be atomic

## The NOT VALID Ratchet Pattern

PostgreSQL provides a mechanism to safely add constraints:

```sql
-- Step 1: Add constraint without validating existing data (instant, non-blocking)
ALTER TABLE users ADD CONSTRAINT users_email_unique UNIQUE (email) NOT VALID;

-- Step 2: New inserts/updates are now validated (the "ratchet" is engaged)
-- Meanwhile, fix any existing violations through backfill/cleanup

-- Step 3: Once all data complies, validate the constraint
ALTER TABLE users VALIDATE CONSTRAINT users_email_unique;
```

This pattern creates a **ratchet**: once engaged, it prevents new violations while giving you time to fix existing ones.

## Mitigation Pattern Details

### DualWrite (Rename Pattern)

To rename `users.email` → `users.email_address`:

1. Add new column `email_address` (Safe)
2. Deploy application that writes to BOTH columns
3. Backfill: `UPDATE users SET email_address = email WHERE email_address IS NULL`
4. Deploy application that reads from new column
5. Deploy application that writes ONLY to new column
6. Drop old column `email` (Destructive, but now safe because nothing uses it)

### Backfill (NOT NULL Pattern)

To add `NOT NULL` to `users.email`:

1. Add CHECK constraint with NOT VALID: `CHECK (email IS NOT NULL) NOT VALID` (Ratchet)
2. Backfill any NULL values
3. Validate: `VALIDATE CONSTRAINT ...`
4. Add actual NOT NULL: `ALTER COLUMN email SET NOT NULL`
5. Drop the CHECK constraint (now redundant)

### Ratchet (Constraint Pattern)

To add `UNIQUE(email)`:

1. Add constraint NOT VALID: `ADD CONSTRAINT ... UNIQUE (email) NOT VALID`
2. Fix any existing duplicates (application-specific logic)
3. Validate: `VALIDATE CONSTRAINT ...`

### Destructive

Drop operations are fundamentally different:
- They're often intentional (cleaning up unused structures)
- They can't be "decomposed" - they're the end state
- But they must be verified safe (nothing references the dropped object)

For drops, the decomposition is temporal:
1. Stop using the object in application code
2. Wait for all old application instances to drain
3. Perform the drop

## Open Questions for Future Work

1. **How do we handle changes that combine multiple categories?**
   - Example: Rename column AND change type simultaneously
   - Likely answer: Decompose into separate changes, each with its own category

2. **Should "Destructive" be further subdivided?**
   - "Intentional removal" vs "Data loss risk"
   - Dropping an unused table is different from dropping a table with data

3. **How do we represent the decomposed migration steps?**
   - This module detects breaking changes
   - A separate module would generate the decomposition
   - What's the interface between them?

4. **What about lock-related concerns?**
   - Some operations require `ACCESS EXCLUSIVE` locks
   - Adding an index without `CONCURRENTLY` blocks writes
   - Is this a separate axis of classification?

## References

- [PostgreSQL: Adding NOT VALID constraints](https://www.postgresql.org/docs/current/sql-altertable.html)
- [Strong Migrations gem (Ruby)](https://github.com/ankane/strong_migrations) - similar concept in Rails ecosystem
- [Expand-Contract Pattern](https://www.martinfowler.com/bliki/ParallelChange.html) - Martin Fowler on parallel change
