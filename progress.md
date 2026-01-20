# Breaking Change Detection - Design Progress

## Current State

We have an initial implementation of breaking change detection in `src/db/diff/breaking.rs`. This module analyzes schema diffs and classifies changes by their potential impact on running applications.

### What Was Built

- `ChangeSeverity` enum with three levels: `NonBreaking`, `Warning`, `Breaking`
- `BreakingChangeKind` enum with 16 specific change types
- `BreakingChange` struct with kind, severity, and human-readable description
- `BreakingChangeAnalysis` aggregator with query methods
- `analyze_breaking_changes()` function that walks a `NamespaceDiff`
- Type change classification (safe widening vs dangerous narrowing)
- 151 passing tests

## Design Problem Identified

The `Warning` severity category is flawed. The original thinking was:

> "Adding constraints might fail depending on existing data, so they're warnings rather than breaking changes."

This is incorrect. **If a migration might fail, it IS breaking.** You cannot deploy it with confidence. The distinction between "definitely fails" and "might fail" is not meaningful from a deployment safety perspective.

### The Core Insight

A constraint addition like `ALTER TABLE users ADD CONSTRAINT users_email_unique UNIQUE (email)` is breaking because:

1. It will fail immediately if existing data has duplicates
2. Even if it succeeds today, there's a race condition - new data might create duplicates between when you tested and when you deployed
3. You cannot safely deploy it without additional coordination

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

This pattern creates a **ratchet**: once engaged, it prevents new violations while giving you time to fix existing ones. The ratchet is the key mechanism that makes the migration safe.

## Proposed Redesign: Mitigation Strategies

Rather than classifying by "severity," we should classify by **what kind of process is required to safely execute the change**. This directly supports the goal of decomposing breaking changes into safe steps.

### Proposed Categories

| Category | Description | Examples |
|----------|-------------|----------|
| **Safe** | No mitigation needed. Can deploy directly. | Add nullable column, add index, drop constraint, add enum value |
| **DualWriteRequired** | Requires a period where both old and new structures coexist with synchronized writes. | Rename column, rename table, change column type |
| **BackfillRequired** | Requires populating data before the change can complete. | Add column with NOT NULL (add nullable → backfill → set NOT NULL) |
| **RatchetRequired** | Requires the NOT VALID + backfill + VALIDATE pattern. | Add UNIQUE/CHECK/FK/PK constraint |
| **Destructive** | Intentionally removes data or structure. May be desired, but is irreversible. | Drop table, drop column, remove enum value |

### Why This Is Better

1. **Actionable**: Each category implies a specific decomposition strategy
2. **Binary safety**: A change is either Safe or it requires mitigation - no ambiguous middle ground
3. **Time-aware**: Acknowledges that some changes fundamentally cannot be atomic
4. **Pattern-based**: Maps directly to known PostgreSQL migration patterns

### Mitigation Pattern Details

#### DualWriteRequired (Rename Pattern)

To rename `users.email` → `users.email_address`:

1. Add new column `email_address` (Safe)
2. Deploy application that writes to BOTH columns
3. Backfill: `UPDATE users SET email_address = email WHERE email_address IS NULL`
4. Deploy application that reads from new column
5. Deploy application that writes ONLY to new column
6. Drop old column `email` (Destructive, but now safe because nothing uses it)

#### BackfillRequired (NOT NULL Pattern)

To add `NOT NULL` to `users.email`:

1. Add CHECK constraint with NOT VALID: `CHECK (email IS NOT NULL) NOT VALID` (Ratchet)
2. Backfill any NULL values
3. Validate: `VALIDATE CONSTRAINT ...`
4. Add actual NOT NULL: `ALTER COLUMN email SET NOT NULL`
5. Drop the CHECK constraint (now redundant)

#### RatchetRequired (Constraint Pattern)

To add `UNIQUE(email)`:

1. Add constraint NOT VALID: `ADD CONSTRAINT ... UNIQUE (email) NOT VALID`
2. Fix any existing duplicates (application-specific logic)
3. Validate: `VALIDATE CONSTRAINT ...`

#### Destructive

Drop operations are fundamentally different:

- They're often intentional (cleaning up unused structures)
- They can't be "decomposed" - they're the end state
- But they must be verified safe (nothing references the dropped object)

For drops, the decomposition is temporal:
1. Stop using the object in application code
2. Wait for all old application instances to drain
3. Perform the drop

## Open Questions

1. **Should we track the specific mitigation pattern on each BreakingChangeKind?**
   - Pro: Makes the required steps explicit
   - Con: Adds complexity, patterns may vary by context

2. **How do we handle changes that combine multiple categories?**
   - Example: Rename column AND change type simultaneously
   - Likely answer: Decompose into separate changes, each with its own category

3. **Should "Destructive" be further subdivided?**
   - "Intentional removal" vs "Data loss risk"
   - Dropping an unused table is different from dropping a table with data

4. **How do we represent the decomposed migration steps?**
   - This module detects breaking changes
   - A separate module would generate the decomposition
   - What's the interface between them?

5. **What about lock-related concerns?**
   - Some operations require `ACCESS EXCLUSIVE` locks
   - Adding an index without `CONCURRENTLY` blocks writes
   - Is this a separate axis of classification?

## Next Steps

1. Refactor `ChangeSeverity` to use mitigation-based categories
2. Remove the `Warning` level entirely - changes are Safe or they require mitigation
3. Add `MitigationStrategy` enum that describes HOW to decompose each breaking change
4. Consider whether the `BreakingChangeKind` variants need restructuring to align with mitigation strategies
5. Update tests to reflect the new classification

## References

- [PostgreSQL: Adding NOT VALID constraints](https://www.postgresql.org/docs/current/sql-altertable.html)
- [Strong Migrations gem (Ruby)](https://github.com/ankane/strong_migrations) - similar concept in Rails ecosystem
- [Expand-Contract Pattern](https://www.martinfowler.com/bliki/ParallelChange.html) - Martin Fowler on parallel change
