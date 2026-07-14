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
  `multiple_items` flag and negotiation `status` — see below), Transfers,
  Inventory, Outbound, Reports (on-screen + CSV export + PDF export via
  `typst-as-lib`, fallback path per `stack-plan.md` risk note — no
  `typst-bake`), Settings (category + document label CRUD, screenshot
  capture command).
- **Backup:** the manual "backup now" button required by `SPEC.md §2` is
  wired up — `SettingsView::show_backup_panel` (`src/ui/views/settings.rs`)
  calls `crate::backup::backup_to_zip`, same path-text-input + Save/Cancel
  pattern as CSV/PDF export in Reports.
- **Dashboard:** currently an empty placeholder. Suggested content is
  documented in `SPEC.md §5.5` — not yet prioritised.
- **No pending features remain approved-but-unimplemented** as of this
  writing — the five sections below (Purchase `multiple_items`, purchase
  negotiation status, inline "+ New donor", permanent itemized inventory
  table, native screenshot capture) are all shipped. Next candidates are
  Dashboard content (optional, `SPEC.md §5.5`) or whatever's raised fresh.

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

## Purchase negotiation status (implemented)

A purchase can be recorded as `negotiating` to capture an in-progress
deal (e.g. an active Kleinanzeigen chat) without committing it to the
EUR/BRL ledger until confirmed. Status lives on **`purchase`**, not
`inventory_item` — `inventory_item.status` is a separate, unrelated
closed enum (`available`/`reserved`/`donated`), and the ledger write was
already atomic with `purchase` insert/update, so gating it on the
purchase's own lifecycle was the smaller, more localized change.

- `purchase.status` (`negotiating` | `bought`), CHECK-constrained TEXT,
  default `bought` — preserves prior behavior for the common
  buy-outright case. Added via
  `migrations/003_purchase_negotiation_status.sql`; `schema.sql` kept in
  sync. A "Start as negotiating" checkbox on the purchase form
  (`src/ui/views/purchases.rs`) opts into the deferred flow.
- `negotiating`: purchase row inserted, **no** ledger row written
  (`purchases::insert`/`update` in `src/db/queries/purchases.rs` gate
  the `eur_transaction`/`brl_transaction` insert on `status`). No
  inventory item can be created against it — `show_purchase_source` in
  `src/ui/views/inventory.rs` excludes negotiating purchases from the
  source picker entirely, not just greys them out.
- `negotiating → bought`: a dedicated "Mark as bought" button
  (`src/ui/views/purchases.rs`) triggers the first-ever ledger write.
  `bought` is terminal — `purchases::update` fetches the row's current
  status and forces it to stay `bought` even if a stale draft claims
  otherwise, so it can never revert.
- Dropping a negotiating purchase hard-deletes the row —
  `purchases::delete` scopes the `DELETE` to `status = 'negotiating'` in
  the query itself, the codebase's only record-level hard-delete. Any
  documents already attached are soft-deleted first
  (`drop_negotiating_purchase` in `src/ui/views/purchases.rs`), never
  orphaned or hard-deleted alongside the purchase row. Documented as the
  explicit §2 exception in SPEC.md §3.6.
- Ledger totals and per-donor reports need no query changes: a
  negotiating purchase never has a ledger row, so it's excluded
  automatically.
- 8 DB-layer tests in `src/db/queries/purchases.rs` cover the full
  status lifecycle (including a regression test for the pre-existing
  bought-edit-recreates-ledger behavior); a migration-chain test in
  `src/db/mod.rs` confirms the new column applies cleanly through the
  real `rusqlite_migration` path.

## Inline "+ New donor" from Inventory's donation sub-form (implemented)

Add item → source = Donation → "+ New donation" (`show_donation_source`
in `src/ui/views/inventory.rs`) now has a "+ New donor" escape hatch
next to its Donor `ComboBox`, so "add item → new donation → new donor"
is one pass without leaving Inventory.

Same shape as the two precedents it sits alongside: `outbound.rs`'s
"+ New recipient project" and `inventory.rs`'s own "+ New donation"
(the very sub-form this extends one level deeper) — a `ComboBox` + a
button setting a local `Option<SomeDraft>` field, an inline
`egui::Group` shown when that field is `Some`, a deferred local `enum`
applying Create/Cancel after the borrow split, then an `insert` query
whose new id gets wired straight back into the parent draft.

- New `InventoryView` field `new_donor: Option<DonorDraft>`, reset
  alongside `new_donation` at every one of its existing reset points
  (Add item button, list-row click, form Cancel, and the donation
  sub-form's own Create/Cancel).
- Inline group below the Donor `ComboBox`: Name* / Contact info / Notes
  (matching `DonorsView`'s own form and the "+ New recipient project"
  precedent's full field set), Create gated on a non-empty name.
- On Create: `donors_qry::insert` (no new query needed), sets
  `nd.donor_id = Some(new_id)` on the in-progress donation draft,
  clears `new_donor`, sets `donors_loaded = false` to invalidate the
  donor cache — mirrors what "+ New donation"'s own Create handler
  already does for `donations_loaded`.
- `new_donation` and `new_donor` are different `InventoryView` fields,
  so accessing both in the same method is a disjoint-field borrow and
  compiles fine as direct field access — this is not routed through a
  helper method taking `&mut self` as a whole, which would fail to
  split.
- This is the third copy of the "combo + inline create/cancel sub-form"
  shape in this codebase (recipient project, donation, donor). Not
  extracted — the three call sites are entangled with each view's own
  fields in a way a generic shared widget would need real design work
  to abstract cleanly. Reconsider only if a fourth shows up.
- Reviewed by `rust-code-reviewer`: no correctness/lifecycle findings.

## Permanent itemized inventory table in Reports (implemented)

Extends the "aggregate summary above, permanent unfiltered line-by-line
detail table below" pattern already shipped for the EUR ledger, BRL
ledger, Donor Breakdown, and Outbound summary tabs
(`show_eur_running_ledger`, `show_brl_running_ledger`,
`show_donor_activity_log`, `show_outbound_history`) to the Inventory
summary tab — `show_inventory_item_log` in `src/ui/views/reports.rs`,
called at the end of `show_inventory_summary`.

- Sorted chronologically by acquisition date, oldest first, per the
  owner's choice (the alternative — sort by category/name with no join
  — was offered and declined). `InventoryItemRow` had no date field of
  its own, so `InventoryItemRow.acquired_date: Option<String>` was
  added, computed in `inventory::list` from
  `physical_donation.date_received` or `purchase.date` depending on
  `source_type` (both already `LEFT JOIN`ed for the pre-existing
  `source_desc` field — the query just gained `pu.date`).
- Columns: Name / Category / Status / Location / Source, per the
  owner's choice — no visible Date column, even though the sort key
  itself isn't shown. `source_desc` is reused as-is for Source.
- `None` (missing source join — shouldn't happen given the `NOT NULL`
  FK, `PRAGMA foreign_keys = ON` in `src/db/mod.rs`) sorts *before*
  `Some`, surfacing a data-integrity problem at the top of the list
  rather than hiding it at the bottom; commented at the sort site.
- `db::queries::inventory` had no tests before this; added one
  (`acquired_date_comes_from_the_matching_source_table`) covering both
  the donation and purchase date-selection paths, since the query's
  column-index tuple grew from 11 to 14 positions and had nothing
  guarding a mismatch.
- Reviewed by `rust-code-reviewer`: no 🔴 findings; the two 🟡s (missing
  test coverage, `None`-sort ordering) were addressed above.

## Native screenshot capture & filing (implemented)

Source: `NewFeature-PurchaseStatus.md` §3.y. A "Capture screenshot"
button on Inventory items, Purchases, and Transfers invokes an
OS region-select tool and files the result as a labeled document —
same naming convention (SPEC.md §4.2) as drag-and-drop, just sourced
from a screenshot instead of the filesystem.

- New `app_setting` key-value table (`migrations/004_app_setting.sql`)
  — the first generic settings schema in this codebase (previously
  only `category`/`document_label` were DB-backed config). Holds
  `screenshot_command`, a `{path}`-templated command string, seeded
  with an OS-appropriate default (`cfg!(target_os)`, Linux/macOS only
  — no reliable Windows CLI default) on first run via
  `db::seed_default_settings`, only if unset so a user edit is never
  clobbered. Editable in a new Settings panel
  (`SettingsView::show_screenshot_panel`); an explicit blank save is
  allowed (clears/disables capture), a non-blank save without the
  placeholder is rejected.
- `src/screenshot.rs`: `capture()` substitutes `{path}` (quoted, so a
  temp dir containing a space — routine on Windows — doesn't split
  into multiple shell args) into the command and runs it via `sh -c` /
  `cmd /C`. Result classification: shell exit 127 → real error ("tool
  not found"); any other non-zero exit, or the expected file missing,
  → neutral `Cancelled` (not an error — most region-select tools like
  maim/grim+slurp/screencapture -i signal Escape via non-zero exit,
  and the source doc's own two sections disagreed on whether that's
  "cancel" or "failure"; resolved in favor of the source doc's
  Result-handling section, which explicitly calls it cancel).
- `docs_fs::file_document` — the shared "generate filename → copy →
  insert document row" helper this feature was the 4th call site for,
  extracted from three near-identical inline copies in
  `inventory.rs`/`purchases.rs`/`transfers.rs`. On a DB-insert failure
  after the copy already succeeded, it deletes the copied file rather
  than leaving an orphan with no document row.
- `PendingAttachment` gained an `is_temp` flag (true only for
  screenshot-sourced files) so the temp capture gets deleted on
  cancel, on successful attach, and at every form-reset point
  (including `purchases.rs`'s negotiating-purchase drop flow) — not
  left to accumulate in the OS temp dir.
- Deleted `src/ui/widgets/document_panel.rs` (and the now-empty
  `ui/widgets/` module) — a dead, never-called stub confirmed via grep
  before removal; superseded by the inline per-view document panels
  this feature also touched.
- Reviewed by `rust-code-reviewer`: no 🔴 findings; three 🟡s (orphaned
  file on insert failure, unquoted `{path}`, negotiating-drop not
  discarding a pending capture) fixed before commit. Also caught by
  self-review: a flaky test from temp-filename collisions under
  parallel test execution (fixed with an atomic counter).

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
  (including `settings.rs` for category/label/screenshot-command
  management), `reports/` for PDF/CSV generation, `docs_fs.rs` for filename
  generation, the shared document-filing helper, and soft-delete,
  `screenshot.rs` for OS screenshot-tool invocation, `backup.rs` for the
  zip-based backup. No `ui/widgets/` — the one stub it ever held
  (`document_panel.rs`) was deleted unused; add it back only if a second
  shared widget actually materializes.
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
