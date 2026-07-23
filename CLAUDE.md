# CLAUDE.md

Project memory and working instructions for Claude Code in this repository.

## What this project is

A Rust application for a small charitable project that tracks physical
skateboard-related donations, dual-currency (EUR/BRL) cash flow, purchases, and
outbound donations to recipient projects in Brazil.

**Two front-ends, one domain core.** The original egui desktop app remains the
primary interface. A web front-end is being added so a second, occasional user
can access the same data from another machine on the LAN. Both binaries run on
the same internal-LAN Linux machine against the same SQLite database and the
same `documents/` folder. Simultaneous use is possible but expected to be rare.

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
- **In progress: workspace restructure + web front-end — phases 1–4 of 6
  done, on `main`.** See "Workspace restructure and web front-end" below
  for the full phase list, what each one did, and backlog items its review
  surfaced. Remaining: phase 5 (web crate) and phase 6 (deployment). The
  desktop app has kept working and behaving identically at every phase
  boundary so far. Tag `v1.0-desktop` marks the pre-restructure state.

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

## Workspace restructure and web front-end (in progress)

Goal: extract the domain layer into a shared crate so a web front-end can
be added alongside the existing desktop app, both running on the same
internal-LAN machine against one database.

### Target layout

```
adm-sfa/
  Cargo.toml          # workspace
  crates/
    core/             # model/, db/, schema.sql, migrations/, docs_fs, backup, config
    reports/          # csv + typst rendering (pure renderers, no aggregation)
    desktop/          # existing egui UI + screenshot.rs
    web/              # axum + server-rendered templates (new)
```

`desktop` and `web` both depend on `core` and `reports`. Neither knows the
other exists.

**`core`'s actual package name is `adm_sfa_core`, not the literal string
`core`.** Naming a workspace crate `core` shadows Rust's own sysroot `core`
crate in the extern prelude of every crate that depends on it — confirmed
during phase 1 with a live repro (`use core::mem;` inside `desktop` silently
resolved to the local crate instead of `::core` once a dependency named
`core` existed, rather than failing loudly). The directory stays
`crates/core/`; only the `[package].name` / `use adm_sfa_core::...` differ
from what the prose above calls it. `web` (phase 5) must depend on it the
same way `desktop` does — `adm_sfa_core = { path = "../core" }`, not
`core = { path = "../core" }`.

### Phases

Work through these in order, one Claude Code session per phase, each
ending in a working desktop app. Do not start a phase before the previous
one compiles, passes tests, and behaves identically.

1. **Workspace split.** Mechanical move only — no logic or signature
   changes beyond what visibility requires. Fold in the configurable data
   root, and enable WAL mode, here. **Checkpoint:** if types resist moving
   because `ui/` is threaded into them, stop and report rather than
   working around it — that finding changes the plan.
2. **Invariants into `core`.** Push down the cross-row rules currently
   enforced only in view code (see "Known domain-logic-in-view debt"
   below). Highest priority: the outbound item status guard, which is a
   real data-integrity gap today and an exploitable one once an HTTP
   client can call it.
3. **Reports aggregation into `core`.** Extract `build_donor_rows`, the
   EUR/BRL summary folds, `build_audit_entries`, and the free functions
   (`in_range`, the `*_tx_description` helpers, `donor_or_anonymous`) out
   of `ui/views/reports.rs`. Unify the three `compute_balance` copies into
   one. **Verification:** generate every report before and after and diff
   the output — byte-identical, or explain the difference. **Done** — see
   `crates/core/src/reporting.rs`; a temporary snapshot test confirmed
   byte-identical output before/after, then was replaced by 14 focused
   unit tests on the extracted functions. Reviewed by `rust-code-reviewer`
   (no 🔴 findings; the two 🟡s are logged below, not yet fixed).
4. **Service layer.** Operation-shaped functions in `core`
   (`create_purchase`, `mark_purchase_bought`, `donate_items`,
   `attach_document`, …). Desktop views call these instead of reaching
   into `db::queries` directly. **Done** — see `crates/core/src/service.rs`
   plus `docs_fs::remove_document` (a real ordering fix: soft-deleting a
   document now moves the file *before* marking the DB row deleted, not
   after — the old order could leave an orphaned live file permanently
   unreachable from the UI, silently overwritable by a later upload
   reusing the same generated filename) and a new `outbound::require_gift`
   guard (an event needs at least one item or cash, previously only
   enforced by the desktop Save button). Reviewed by `rust-code-reviewer`;
   the one 🟡 that mattered (doc-removal errors had regressed to hardcoded
   English, losing German/Portuguese translation) was fixed before commit
   — the rest are logged below, not fixed.
5. **Web crate.** axum + server-rendered templates over the service layer.
   Multipart upload replacing drag-and-drop; file serving for
   `documents/`; single shared password + session cookie.
6. **Deployment.** systemd unit, own `adm-sfa` user, `WorkingDirectory` at
   the data root, hardening (`PrivateTmp`, `ProtectSystem=strict`,
   `ReadWritePaths` scoped to the data dir, `NoNewPrivileges`), bind to
   the LAN IP with a firewall rule scoped to the subnet,
   `WantedBy=multi-user.target` (the machine is not always on). Nightly
   `sqlite3 .backup` + rsync of `documents/` off the machine.

### Known domain-logic-in-view debt (phase 2 — fixed)

From an audit of the pre-restructure codebase. These were cross-row
invariants with no DB constraint or query-layer guard behind them —
all three now fixed in `crates/core/src/db/queries/{outbound,purchases,
inventory}.rs`, reviewed by `rust-code-reviewer` (no 🔴 findings):

- ~~`db/queries/outbound.rs::link_items` unconditionally sets any passed
  item id to `donated` with no status check.~~ **Fixed**: `link_items` now
  rejects any item that isn't currently `available`, inside the same
  transaction as the event insert/update, so a rejection rolls back
  everything (event row, prior releases, earlier-in-loop links too) —
  verified against `rusqlite::Transaction`'s drop-rolls-back-by-default
  behavior, not assumed.
- ~~"Can't unset `multiple_items` while >1 inventory items are linked" is
  checked inside the Save button's click handler in `purchases.rs`.~~
  **Fixed**: `purchases::update` now calls a new
  `multiple_items_unset_conflict` authoritatively before writing; the
  desktop view's pre-save check calls the same function for its message
  instead of re-implementing the condition.
- ~~`purchase_source_conflict` ("a single-item purchase backs at most one
  inventory item") is implemented *twice, independently*, in
  `inventory.rs`.~~ **Fixed**: collapsed to one shared predicate in the
  view (`purchase_source_blocked`, used by both the picker's grey-out and
  the pre-save check) plus a new authoritative DB-backed
  `inventory::purchase_source_conflict` wired into `insert`/`update`.

**New backlog item found during phase 2's review, confirmed and widened by
manual testing after the phase 2 commit** (not fixed — pre-existing, adjacent
to but not covered by the `link_items` guard above): a `donated` inventory
item has **no locked fields at all** in the edit form
(`crates/desktop/src/ui/views/inventory.rs`) — not just `status` (the
originally-flagged case: editing it back to `available` lets the item be
re-linked to a *second* outbound event, producing two donation records for
one physical item), but every other field too, including reassigning the
item's `source_type`/`source_donation_id`/`source_purchase_id` entirely
after the fact. Needs a deliberate decision before phase 5 exposes this over
HTTP: what should stay editable on a `donated` item (notes? category?) versus
what should lock (status; source; anything that feeds a ledger/reconciliation
figure) — full lock, or an intentional manual-override escape hatch with a
confirmation step. Not blocking any current phase; flagged here so it isn't
lost.

**Related bug found and fixed during the same manual testing pass**: switching
an item's source-type radio button (Donation ↔ Purchase) in the edit form
left the *other* type's id field stale instead of clearing it — e.g.
switching a Purchase-sourced item to Donation kept its old
`source_purchase_id` set, so the DB ended up with `source_type = 'donation'`
*and* a `source_purchase_id` still pointing at the old purchase. That stale
id is exactly what `purchases::linked_item_count` (and this phase's new
`multiple_items_unset_conflict`) counts against, so an unrelated purchase
could appear permanently "linked" even after every item claiming it had been
reassigned elsewhere. Fixed by clearing the other type's id on
`.changed()` for either radio button.

~~Also: `compute_balance` is defined identically in `eur_ledger.rs` and
`brl_ledger.rs`, with a third period-scoped variant inline in
`reports.rs`.~~ **Fixed in phase 3**: unified into one generic
`reporting::compute_balance(flows: impl Iterator<Item = (bool, Decimal)>)`,
called by both ledger views and by the new `eur_summary`/`brl_summary`.
`transfers.rs` recomputes `eur * rate = brl` as a preview label; the
authoritative version is in `db/queries/transfers.rs` and stays there —
this one was never a duplication problem, just a UI preview, so it's out
of scope for both phases.

**New backlog items found during phase 3's review** (not fixed — test
coverage gaps in the highest-risk part of that phase, the aggregation
arithmetic, per `rust-code-reviewer`):
- `crates/core/src/reporting.rs`'s `eur_summary` has a dedicated test for
  the pre-range `starting_balance` calculation
  (`eur_summary_starting_balance_is_the_pre_range_running_total`);
  `brl_summary` doesn't, even though it's an independently-typed copy of
  the exact same filter-then-`compute_balance` pattern. Add the BRL
  equivalent.
- `build_audit_entries`'s doc-count lookup is only tested via the
  `linked_purchase_id` branch; the `linked_transfer_id` branch
  (`EurTxType::TransferToBrlOut` / `BrlTxType::TransferIn`) has no test.

**New backlog items found during phase 4's review** (not fixed — per
`rust-code-reviewer`):
- `service::drop_negotiating_purchase`'s test only covers the happy path,
  not the partial-failure/stop-on-first-error behavior that's the entire
  reason it was extracted (confirmed correct by reading the code — the `?`
  in its per-document loop short-circuits before the purchase row gets
  deleted — just not exercised by a test that induces a mid-loop failure).
- `outbound.rs`'s edit path still calls `db::queries::outbound::update`
  directly rather than through a `service::*` wrapper — only the create
  path got `donate_items`. Both run the identical `require_gift` guard, so
  this is a minor asymmetry against phase 4's "views call service functions
  instead of `db::queries` directly" goal. Decide whether a
  `service::update_donation` should exist before `web` needs the same
  operation, or whether `update` staying un-wrapped is fine.
- `docs_fs::generate_filename`'s collision check only consults currently
  *active* filenames for a record, not anything already in `_deleted/` — so
  re-attaching a same-day, same-default-label document after removing the
  original can regenerate the same filename, which then hits
  `document.filename`'s `UNIQUE` constraint (schema-wide, not scoped by
  `deleted`) and surfaces a confusing raw SQLite error instead of silently
  overwriting anything (verified: no data-loss risk, just a bad error
  message for a plausible legitimate action). Pre-existing, not introduced
  by phase 4 — `remove_document`'s idempotency check just happened to be
  the thing that surfaced it during review. Fix direction: either have
  `generate_filename` also consult `_deleted/` filenames, or namespace
  `_deleted/` by document id so collisions can't occur at all.

What the audit found *correct* and not to be "improved" during the move:
`db/queries/*` (parameterized, no business logic), `model/*` (enum
`label()`/`as_str()`/`is_inflow()` helpers are domain vocabulary, correctly
placed), `src/reports/{csv,pdf}.rs` (pure renderers), and the CRUD-only
views (`donors.rs`, `settings.rs`).

### Platform differences between front-ends

- **Native screenshot capture (`SPEC.md §3.y`) is desktop-only** — a
  permanent platform constraint, not an unimplemented gap. A browser
  cannot invoke the OS screenshot tool on the *client* machine. `web` gets
  plain file upload for the same document labels. `screenshot.rs` stays in
  `crates/desktop`; do not attempt to move it to `core` or reimplement it
  server-side.
- Drag-and-drop attachment becomes HTTP multipart in `web`. The filename
  convention (SPEC.md §4.2) is unchanged and generated in `core` either
  way — the user still never types a filename.
- PDF export (`typst-as-lib`) runs server-side in `web` and returns a
  download response.

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
5. While the workspace restructure is in progress, respect the phase
   boundaries above. Do not opportunistically fix domain-logic-in-view
   debt during phase 1 — the whole point of a mechanical move is that a
   behaviour change can't hide inside it. Report anything you notice and
   leave it for its phase.
6. When adding a feature after phase 5, implement it in `core` first, then
   wire up *both* front-ends — or state explicitly that it's
   platform-specific and why. Silently shipping a feature to only one
   front-end is the failure mode to avoid.

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
- **Two users, one machine, no sync.** *(Supersedes the previous "single
  user" constraint as of the web front-end work.)* The web front-end gets a
  single shared password with a session cookie — not per-user accounts,
  roles, or permissions. Both binaries open the same SQLite file directly;
  SQLite runs in WAL mode. Do not add sync, replication, per-user identity,
  or an ORM/connection-pool layer to "solve" concurrency — WAL plus rare
  overlapping use is the whole design. Stale reads in a long-open desktop
  session are accepted, not worked around.
- **Business rules live in `core`.** UI crates (`desktop`, `web`) call into
  `core` and never implement domain logic, validation, or cross-row
  invariants themselves. If a rule can be violated by a caller that isn't
  the UI, it belongs in `core` — this is what makes the web front-end safe,
  since an HTTP client is untrusted in a way an egui widget was not.
- **`core` takes its data root as configuration.** Never derive the DB or
  `documents/` path from the binary's own location, and never assume the
  two front-ends resolve it differently — they must point at the same root.
- **T1 — The database is monolingual.** No stored value changes meaning or
  spelling based on the active UI locale (see SPEC.md §6). Locale affects
  presentation only.
- **T2 — No user-visible string is hardcoded** in view code. Every one
  resolves through the i18n layer.
- **T3 — Report generators never read `ui_locale`.** Locale is always an
  explicit argument to report generation (SPEC.md §6.3), never read
  implicitly from the UI language setting.
- **T4 — Filenames are locale-independent** (SPEC.md §4.2, §6.1). Already
  true today; this constraint exists to keep it true as the codebase grows.
- **T5 — A missing translation falls back to English and is visible**, not
  silently blank. Fallback must not panic.
- **T6 — CSV output is German-format and locale-independent** (SPEC.md
  §6.4): `;` delimiter, `,` decimal separator, `.` thousands separator,
  regardless of the active UI language.
- **T7 — Amount *input* parsing is never coupled to `ui_locale`** (SPEC.md
  §6.5). The existing comma-or-period leniency (§2) stays available in every
  UI language; input leniency and display formatting are separate concerns.

## Conventions once code exists

- **Module layout** (see `stack-plan.md` for the full tree; paths below are
  post-restructure — before phase 1 they all sit under a single `src/`):
  in `crates/core`: `db/` for `rusqlite` access and query modules per
  entity group, `model/` for plain structs mirroring DB rows (including
  `category` and `document_label` as first-class models, not enums),
  `docs_fs.rs` for filename generation, the shared document-filing helper,
  and soft-delete, `backup.rs` for the zip-based backup, plus the service
  layer (phase 4) and the extracted reports aggregation (phase 3).
  In `crates/reports`: PDF/CSV rendering only — no aggregation.
  In `crates/desktop`: `ui/views/` for one file per section (including
  `settings.rs` for category/label/screenshot-command management) and
  `screenshot.rs` for OS screenshot-tool invocation. No `ui/widgets/` —
  the one stub it ever held (`document_panel.rs`) was deleted unused; add
  it back only if a second shared widget actually materializes.
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

Pre-restructure (single crate):

```sh
cargo build                        # compile (debug)
cargo run                          # run with default data dir (~/.local/share/adm-sfa/)
cargo run -- --data-dir /tmp/test  # run with an alternate data dir (useful for dev/testing)
cargo test                         # run unit tests
cargo clippy -- -D warnings        # lint; treat warnings as errors
cargo fmt                          # auto-format all source files
cargo check                        # fast type-check without producing a binary
```

Post-restructure (workspace) — the `--workspace` / `-p` forms:

```sh
cargo build --workspace                      # compile everything
cargo run -p desktop                         # run the egui app
cargo run -p desktop -- --data-dir /tmp/test # alternate data dir
cargo run -p web                             # run the web server (phase 5+)
cargo test --workspace                       # run all tests across crates
cargo clippy --workspace -- -D warnings      # lint everything
cargo fmt --all                              # format all crates
```
