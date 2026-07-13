# CLAUDE.md

Project memory and working instructions for Claude Code in this repository.

## What this project is

A desktop application (Rust) for a small charitable project that tracks physical
skateboard-related donations, dual-currency (EUR/BRL) cash flow, purchases, and
outbound donations to recipient projects in Brazil. Single user, single machine,
no sync required.

**Read `SPEC.md` before making any architectural or data-model decision.** It is
the source of truth for entities, fields, the document/filename system, and
reporting requirements. It was produced through detailed back-and-forth with the
project owner — treat it as settled unless explicitly told otherwise in this
session. Do not redesign the data model from scratch; extend or implement what's
there.

## Status

- Functional specification: complete (`SPEC.md`).
- Rust stack: **decided.** See `stack-plan.md` for the chosen storage layer
  (`rusqlite` + `rusqlite_migration`), UI framework (`egui`/`eframe`), PDF
  library (`typst-as-lib` + `typst-bake`, with a fallback noted if the bake
  step proves too unstable), backup mechanism (`zip` + `walkdir`), and the
  first-draft `schema.sql`. Section 6 of `SPEC.md` is left unchanged as
  historical context on what was deferred — `stack-plan.md` is the current
  source of truth for tech choices.
- **Implementation: substantially complete.** All SPEC.md sections are
  implemented: Donors, EUR Ledger, BRL Ledger, Purchases (including the
  `multiple_items` flag — see below), Transfers, Inventory, Outbound,
  Reports (on-screen + CSV export + PDF export via `typst-as-lib`,
  fallback path per `stack-plan.md` risk note — no `typst-bake`), Settings
  (category + document label CRUD).
- **Known gap:** the manual "backup now" button required by `SPEC.md §2`
  is not wired up. `src/backup.rs::backup_to_zip` is fully implemented
  (zips the data dir to a dest path) but is currently `#[allow(dead_code)]`
  — no UI calls it yet.
- **Dashboard:** currently an empty placeholder. Suggested content is
  documented in `SPEC.md §5.5` — not yet prioritised.

## Purchase `multiple_items` flag (implemented)

A boolean `multiple_items` on the `purchase` table controls whether a
purchase may be linked to more than one inventory item.

- `multiple_items = false` (default): the purchase can only appear as
  `source_purchase_id` on exactly one `inventory_item`. The inventory
  source picker (`show_purchase_source` in `src/ui/views/inventory.rs`)
  greys out / excludes single-item purchases that already have one item
  linked, and validates on save.
- `multiple_items = true`: no limit — multiple inventory items can share
  the same purchase (e.g. a lot purchase of several decks).
- Added via `migrations/002_purchase_multiple_items.sql`; `schema.sql`'s
  `purchase` table is kept in sync with this column.

## Pending features (approved, not yet implemented)

Source: `NewFeature-PurchaseStatus.md`. Both reviewed for harmony against
the current implementation before coding starts.

### Purchase negotiation status

Allows a purchase to be recorded at the moment a negotiation begins (e.g.
a Kleinanzeigen chat) without committing it to the EUR/BRL ledger until
the deal is confirmed.

**Design deviates from the source doc**: the source doc puts the new
status on the *item*. Reviewed against the current implementation and
resolved: it belongs on **`purchase`**, not `inventory_item`. Reasons:
(1) `inventory_item.status` is already a closed, unrelated enum
(`available`/`reserved`/`donated`) — reusing it would collide; (2) the
EUR/BRL ledger write is already atomic with `purchase` insert/update
(`src/db/queries/purchases.rs`), not with item creation, so gating the
ledger write on the *purchase's* lifecycle is the smaller, more localized
change; (3) inventory item creation already requires a fully-persisted
purchase to link to (`show_purchase_source` in
`src/ui/views/inventory.rs`) — under this design that requirement is
untouched, since inventory items are still only ever created once a
purchase reaches `bought`.

**Behaviour:**
- New `purchase.status` (`negotiating` | `bought`). Default `bought`, to
  preserve today's behavior for the common case of a purchase entered
  after the fact. A "Start as negotiating" toggle on the purchase form
  opts into the deferred flow for an in-progress deal.
- `negotiating`: purchase row is inserted (channel, seller_info, cost,
  etc. captured as today) but **no** `eur_transaction`/`brl_transaction`
  row is written.
- `negotiating → bought`: single status edit; this transition is what
  triggers the ledger write (mirrors today's insert-time ledger write,
  just moved to the transition).
- `bought` is terminal: reverting `bought → negotiating` is out of scope
  (would require reversing a ledger entry) — disallow in the UI.
- Dropping a negotiating purchase: hard-delete the *purchase row only*.
  Resolved — safe because a negotiating purchase never wrote a ledger
  entry or created an inventory item, so there is no auditable
  financial/inventory state to preserve. This is the codebase's first
  and only record-level delete; keep it narrowly scoped to
  `status = negotiating` (guard the query so a `bought` purchase can
  never hit this path). Documented as the explicit §2 exception in
  SPEC.md §3.6. **Documents attached to the purchase are a separate
  concern**: they must go through the normal document soft-delete path
  first, not be hard-deleted or left orphaned (`document.record_id` is a
  bare `INTEGER`, not an FK — nothing at the DB level stops an orphan) —
  the implementation must soft-delete or reject-if-any-attached before
  hard-deleting the purchase row itself.

**Ledger constraint (non-negotiable, inherited from the source doc):** a
`negotiating` purchase must not appear in any EUR/BRL ledger total or
per-donor breakdown. Satisfied for free by this design — no ledger row
exists until `bought`, so no query changes are needed to keep negotiating
purchases out of ledger totals or reports.

**Implementation notes / harmony conflicts found:**
- `purchases::insert` and `purchases::update`
  (`src/db/queries/purchases.rs:43-116`) currently write the linked
  ledger row unconditionally, and `update()` deletes+recreates it on
  every edit. Both need to become conditional on `status`.
- `show_purchase_source` in `src/ui/views/inventory.rs:491-538` lists
  purchases from `purchases_qry::list` with no status filter today —
  needs to exclude `negotiating` purchases from the source picker (you
  can't create an inventory item against money that isn't committed
  yet).
- Resolved: `purchase.status` is CHECK-constrained TEXT
  (`CHECK (status IN ('negotiating','bought'))`), matching
  `purchase.currency` and `inventory_item.status` already on these
  tables. The source doc's `item_status`/`purchase_status` lookup-table
  suggestion is intentionally overridden — the set is closed and not
  expected to grow, so the first-class-entity convention (which exists
  to allow user-added rows: donors, projects, categories, labels) does
  not apply here.
- SPEC.md §3.6 updated to document `status` (and the previously-
  undocumented `multiple_items`), with the negotiation lifecycle note
  and the §2 hard-delete exception. In sync.

### Native screenshot capture & filing

Source: `NewFeature-PurchaseStatus.md` §3.y. Additive, no design
conflicts found — mostly new capability, not a change to existing
behavior.

**Behaviour:** a "Capture screenshot" button on an item/purchase/transfer
record invokes the OS's native region-select screenshot tool
(config-driven command per OS, seeded default per detected OS), receives
the resulting PNG at a controlled temp path, and files it as a labeled
document via the existing naming convention (SPEC.md §4.2) — same as a
drag-and-dropped file, just sourced from a screenshot instead of the
filesystem. Cancel / non-zero exit → no document, no orphan record, a
neutral "capture cancelled" state.

**Implementation notes / harmony gaps found:**
- No shared "file an already-on-disk path as a document" helper exists
  yet — the drag-and-drop flow (commit `28f1c87`) duplicates the
  generate-filename → copy-to-documents → insert-document-row sequence
  inline in `purchases.rs`, `transfers.rs`, and `inventory.rs`. This
  feature would be a 4th call site — worth extracting a shared helper
  (e.g. in `docs_fs.rs`) at that point rather than duplicating a 4th
  time.
- No generic settings/config table exists (only `category` and
  `document_label` are DB-backed config today). Per-OS capture command
  strings and a default label need new schema — not yet in SPEC.md.
- `std::process::Command` is not used anywhere in this codebase yet —
  this is a new capability with no established error-handling/platform-
  dispatch pattern to follow; needs one designed (missing-tool vs.
  non-zero-exit vs. user-cancel all need distinct, clear handling per the
  source doc's constraints).
- `src/ui/widgets/document_panel.rs` is currently a dead stub (never
  called) — worth checking whether this feature should finally use or
  replace it, or whether it should be deleted as unused.

## How to work in this repo

1. On starting a session, read `SPEC.md` and `stack-plan.md` in full before
   proposing or writing anything.
2. If a requested feature isn't covered by `SPEC.md`, say so explicitly and ask
   rather than inventing new entities or fields silently.
3. The stack is decided — see `stack-plan.md`. Do not re-litigate storage
   layer, UI framework, or PDF library choices without explicit instruction.
   If `schema.sql` / `migrations/001_initial.sql` don't yet exist in the repo,
   create them from `stack-plan.md`'s schema section before writing any other
   code.
4. Keep `SPEC.md`, `stack-plan.md`, and the actual schema/code in sync. If
   implementation reveals an ambiguity or necessary change to the spec or
   plan, flag it and propose an edit rather than letting the code silently
   diverge.

## Non-negotiable design constraints (do not change without explicit confirmation)

- **Two independent cash ledgers** (EUR and BRL). Currency conversion happens
  only at the annual EUR→BRL transfer, with a manually entered exchange rate.
  Never introduce a live FX rate dependency.
- **No hardcoded purchase channels.** Kleinanzeigen is the common case but
  purchases are generic (`channel` as free text + `seller_info` as a generic
  note field), for both EUR and BRL purchases.
- **Donors, recipient projects, and item categories are first-class entities**,
  not free text — required for reporting (esp. per-donor breakdowns). Item
  categories live in a `category` table with an FK from `inventory_item`, not
  a hardcoded enum/CHECK constraint.
- **Document labels are config-driven**, not a hardcoded source-level const —
  they live in a `document_label` table, seeded at migration time, so adding
  a label is a row insert. `document.label` itself stores the name as TEXT
  (not an FK) so historical documents stay valid if a label is later renamed
  or retired.
- **Documents are file-on-disk, not BLOBs**, stored flat in `documents/` with
  auto-generated filenames (see SPEC.md §4.2) — never prompt the user to type a
  filename.
- **Soft-delete only** for documents — move to `documents/_deleted/`, never
  hard-delete from within the app.
- **Single user.** Do not add multi-user auth, sync, or concurrent-write
  handling — out of scope.

## Conventions once code exists

- **Module layout** (see `stack-plan.md` for the full tree): `db/` for
  `rusqlite` access and query modules per entity group, `model/` for plain
  structs mirroring DB rows (including `category` and `document_label` as
  first-class models, not enums), `ui/views/` for one file per section
  (including `settings.rs` for category/label management), `ui/widgets/` for
  shared components (`document_panel.rs`, `amount_field.rs`), `reports/` for
  PDF/CSV generation, `docs_fs.rs` for filename generation and soft-delete,
  `backup.rs` for the zip-based backup.
- **Migrations**: `rusqlite_migration`, tracked via `schema.sql` (canonical,
  hand-maintained) kept in sync with `migrations/NNN_name.sql` (applied,
  incremental). New tables/columns get a new migration file, not edits to
  `001_initial.sql` once it's applied anywhere.
- **Money**: `rust_decimal`, stored as TEXT in SQLite — never use SQLite
  NUMERIC/REAL for money fields.
- **Table declaration order** in schema files should stay forward-reference
  free where practical (a table shouldn't `REFERENCES` a table declared later)
  even though SQLite itself resolves FK targets at DML time — keeps the
  schema portable to stricter tooling/linters.
- **Dependency versions**: check `stack-plan.md`'s pinned versions before
  adding a new dependency version bump; re-verify against crates.io if it's
  been a while since the plan was written, especially for `typst-as-lib` /
  `typst-bake` (both explicitly unstable upstream) and anything with a large
  major-version gap.

## Code review
After completing any feature or fix and before committing, delegate the
changed code to the `rust-code-reviewer` subagent. Address 🔴 findings
before the commit; surface 🟡/🟢 for me to decide.


## Useful commands

```sh
cargo build                        # compile (debug)
cargo run                          # run with default data dir (~/.local/share/adm-sfa/)
cargo run -- --data-dir /tmp/test  # run with an alternate data dir (useful for dev/testing)
cargo test                         # run unit tests
cargo clippy -- -D warnings        # lint; treat warnings as errors
cargo fmt                          # auto-format all source files
cargo check                        # fast type-check without producing a binary
```
