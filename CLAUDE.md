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
- **All 6 planned phases of the workspace restructure + web front-end are
  done, on `main`**, and phase 5's originally-deferred backlog (the
  remaining seven of `web`'s nine sections) has since been completed too,
  on branch `phase-5-web-remaining-sections` — see "Workspace restructure
  and web front-end" below for the full phase list, what each one did, and
  backlog items its reviews surfaced. `web` now has full section parity
  with `desktop`: Purchases, Donors, EUR Ledger, BRL Ledger, Transfers,
  Inventory, Outbound, Reports (on-screen + CSV/PDF export), and Settings
  (category/label CRUD; locale picker, screenshot command, and manual
  backup stay desktop-only, each for a documented reason). One thing
  remains deliberately open, not an oversight: phase 6 shipped deployment
  *templates* (`deploy/`) — nothing has actually been installed on a real
  machine from within a session, since that requires editing system
  files/creating a system user/configuring a firewall on a specific host.
  The desktop app has kept working and behaving identically at every phase
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
   `documents/`; single shared password + session cookie. **Done, full
   section parity** — see `crates/web/` (new binary `adm-sfa-web`, Askama
   0.12 templates under `crates/web/templates/`). Shipped in two passes:
   first Purchases (the "rich" section: full CRUD, negotiating→bought
   lifecycle via `service::mark_purchase_bought`, document upload/removal
   via `service::attach_document`/`docs_fs::remove_document`) and Donors (a
   simple CRUD section), as an explicit scope reduction ("skeleton + 1–2
   sections first" over "full parity in one session"); then, in a later
   session on branch `phase-5-web-remaining-sections`, the remaining seven
   — EUR Ledger, BRL Ledger, Transfers, Inventory, Outbound, Reports,
   Settings — each its own reviewed commit. Auth: single shared password
   from `ADM_SFA_WEB_PASSWORD` (constant-time compare via `subtle`), signed
   session cookie (`axum-extra`'s `SignedCookieJar`, `HttpOnly` +
   `SameSite=Strict`, no per-user identity — matches the "two users, one
   machine, no sync" constraint). `AppState.db` is a single `Connection`
   behind a plain `Mutex` (not a pool), matching `desktop`'s
   single-`Connection` shape; `AppState::conn()` recovers from mutex
   poisoning rather than panicking the whole server on one bad request.
   Binds to `127.0.0.1:8080` by default (LAN-wide binding deferred to phase
   6).

   The first pass was reviewed by `rust-code-reviewer` with no 🔴 findings;
   three 🟡s (silent error-swallowing in `mark_bought`/`remove_document`/
   `attach_document`'s no-file case, temp-upload filename collisions under
   concurrent requests, mutex-poison panic risk) were fixed before commit.
   The second pass's seven sections were each reviewed independently, and
   three real 🔴-equivalent gaps were found and fixed along the way — all
   in the same category: a business rule that was previously enforced only
   in `desktop`'s UI (or not enforced anywhere authoritative at all), now
   reachable by an untrusted HTTP client for the first time because `web`
   gained a route that calls the same `core` query directly:
   - `eur_ledger::insert`/`update` and `transfers::insert`/`update` had no
     amount-positivity check in `core` — only desktop's Save-button
     gating. A crafted POST could write a negative or zero "donation"
     straight into the ledger. Fixed in `core::db::queries::{eur_ledger,
     transfers}`, with regression tests. The `transfers` fix also caught a
     `Decimal` multiplication overflow panic (`eur_amount * rate` used the
     panicking `*` operator with no ceiling on either input) — switched to
     `checked_mul` in both `core` and the web route's own preview label.
   - `inventory::check_purchase_source` never checked a purchase's
     `negotiating`/`bought` status, only `multiple_items` and existing
     links — both `desktop`'s and `web`'s pickers already excluded
     negotiating purchases from the dropdown, but that was UI-only. A
     crafted POST could link an inventory item straight to a negotiating
     purchase (no ledger row, cost unconfirmed). Fixed authoritatively in
     `core::db::queries::inventory::check_purchase_source`, with
     regression tests; verified live that the same crafted request is
     rejected over HTTP now.
   - `crates/web/src/routes/reports.rs::export_csv` spliced the `tab`
     query param into a temp file path with only an emptiness check, not
     the same `TABS` allowlist the on-screen route already used — a
     crafted `tab=../../../whatever` could steer `reports::csv::write`'s
     destination outside the OS temp dir. Fixed by validating `tab` the
     same way both routes now; verified live that the exploit payload
     writes nothing outside the temp dir. The same review pass also added
     the crate's established `AtomicU64` per-request-uniqueness pattern to
     the report-export temp filenames (previously only disambiguated by
     tab + process id, a collision risk under concurrent exports) and
     stopped silently swallowing a failed temp-file read into an empty
     download.

   One 🟡 surfaced in the Settings review and deliberately left open, not
   fixed: `categories::insert`/`update` and `documents::insert_label`/
   `update_label` have no server-side non-empty-name check (only an HTML
   `required` attribute, trivially bypassed) — verified live that a raw
   POST with an empty name inserts a blank category that then shows up in
   every category picker. This is the same shape as an existing,
   previously-unflagged gap in `donors.rs`'s `donor.name` — real, but
   shared pre-existing-shaped debt spanning multiple already-shipped
   sections rather than something unique to Settings, so it's logged below
   rather than fixed as a one-off.
6. **Deployment.** systemd unit, own `adm-sfa` user, `WorkingDirectory` at
   the data root, hardening (`PrivateTmp`, `ProtectSystem=strict`,
   `ReadWritePaths` scoped to the data dir, `NoNewPrivileges`), bind to
   the LAN IP with a firewall rule scoped to the subnet,
   `WantedBy=multi-user.target` (the machine is not always on). Nightly
   `sqlite3 .backup` + rsync of `documents/` off the machine. **Done** —
   artifacts under `deploy/` (`adm-sfa-web.service`,
   `adm-sfa-backup.service` + `adm-sfa-backup.timer`, `backup.sh`); see
   "Deploying the web service" below for the install sequence. These are
   templates with `<PLACEHOLDER>` values (data dir, LAN IP, backup
   destination) — none of it has been applied to a real machine from this
   session, since doing so means editing system files, creating a system
   user, and configuring a firewall on a specific host this session has no
   access to. Verified only via `systemd-analyze verify` against copies
   with placeholders substituted for dummy real paths — confirms the unit
   syntax is sound, not that it behaves correctly once installed for real.
   One decision made explicitly, not assumed: desktop's existing data
   directory stays where it is (its current default under the interactive
   user's home) rather than migrating to a system location like
   `/var/lib/adm-sfa` — the dedicated `adm-sfa` service user gets group
   access to that existing path instead, so there's no one-time data
   migration and desktop's launch command doesn't change.

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

~~**New backlog item found during phase 2's review, confirmed and widened by
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
lost.~~ **Fixed** on `backlog-donated-item-field-locking` (PR #16): owner's
call was full lock, notes-only, no manual-override escape hatch. See
"Donated inventory item field locking" below for the implementation.

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

**New backlog items found during phase 5's review** (the three 🟡s that
*were* fixed before commit are described in the phase 5 summary above, not
repeated here). Status below reflects reality as of `backlog-donated-item-
field-locking` (PR #16) — several of these were fixed by dedicated
`backlog-*` branches (PRs #12–#15) *after* this section was last written,
without this section being updated at the time; that gap is why this note
exists instead of the usual inline strikethrough for each one.

- ~~The multipart upload's `label` field ... is arbitrary free text with no
  allow-list check against `document_label`~~ **Fixed** on
  `backlog-document-label-allowlist` (PR #12): `label` is now validated in
  `core` against `docs_qry::labels(conn)` for all three `attach_document`
  call sites (purchases, transfers, inventory), including a path-traversal
  test.
- **Still open, partially addressed.** `crates/web` had zero automated
  tests at the time this was written; `backlog-web-test-coverage` (PR #14)
  added the first suite (`auth::password_matches`/`require_auth`, login,
  and `purchases.rs`'s `attach_document` edge cases — the highest-value
  targets this item called out), and `backlog-donated-item-field-locking`
  (PR #16) added `inventory.rs` route tests on top. Still no tests for
  `eur_ledger.rs`, `brl_ledger.rs`, `transfers.rs`, `outbound.rs`,
  `reports.rs`, `settings.rs`, or `donors.rs`'s routes.
- **Still open.** The "re-fetch the full list and `.find(|x| x.id == id)`"
  pattern was the motivation for `get(conn, id)` per entity — **fixed** on
  `backlog-get-by-id-dedup` (PR #15), which added `get()` to every entity
  in `core` and switched `web`'s single-row lookups to it. (Note: this only
  replaced the *lookup* pattern; it didn't touch call sites that
  legitimately need the full list, e.g. pickers.)
- **Still open.** `crates/web/src/main.rs::parse_data_dir` still duplicates
  `crates/desktop/src/main.rs`'s hand-rolled `--data-dir` parsing verbatim —
  not yet moved into `adm_sfa_core::config`.
- ~~`/logout` ... is a plain `GET`~~ / ~~session cookie has no explicit
  `Max-Age`~~ **Both fixed** on `backlog-logout-post-and-cookie-expiry` (PR
  #13): `/logout` is now a `POST` (styled as a small form in the header),
  and the session cookie gets an explicit 8-hour `Max-Age` (client-enforced
  only — `require_auth` checks signature, not issued-at, documented inline
  as a known tradeoff).
- ~~Web templates and route handlers use hardcoded English strings — no
  `t!()` calls anywhere in `crates/web`, a deliberate, temporary violation
  of T2, spanning all nine sections.~~ **Fixed** on `backlog-web-i18n`
  (commit `aa9ec4c`): `t!()` wired through every template and route
  handler in `crates/web`. Manually tested per `backlog-web-i18n-tests.md`
  against the full 8-step guide (baseline English, live DB-driven locale
  switching with no restart, all nine sections in German/Portuguese,
  interpolated strings, the two reviewer-caught JS-escaping/placeholder
  bugs, on-screen-vs-export locale independence, and the donated-item-lock
  regression check) — passed, with two things noted rather than fixed as
  part of this branch:
  - **New backlog item, found during manual testing:** the Transfers
    form's "BRL received: R$ ..." preview only appears after a save
    (error re-render or the edit page after a successful create), not
    live as the user types an amount/rate — confirmed as a **pre-existing
    web/desktop parity gap, not an i18n regression**. Desktop's
    `crates/desktop/src/ui/views/transfers.rs` recomputes the preview
    every egui frame from the in-progress draft; `web`'s
    `crates/web/src/routes/transfers.rs::brl_preview` only ever runs
    server-side against whatever draft the current page render already
    has, and `crates/web/templates/transfers/form.html` has no
    client-side JS to recompute it on input. Present since the original
    web Transfers port (`d70b5c0`); the i18n commit only swapped the
    hardcoded English string for a translated one, it didn't add or
    remove any live-preview mechanism. Needs a deliberate decision (add
    a small `oninput` JS recompute, matching values/format the server
    already computes on submit, vs. leave web without a live preview)
    before treating it as done.
  - **Not yet tested, flagged so it isn't silently assumed passing:**
    step 6 of the test guide — confirming the Export CSV/PDF forms'
    "Report language" dropdown produces a download in the *dropdown's*
    chosen language even when it differs from the current on-screen
    `ui_locale` chrome (T3: export locale is always an explicit
    per-export argument, never read from the ambient UI locale). The
    on-screen-vs-chrome-locale half of step 6 *was* tested and passed;
    only the dropdown-vs-download-language half was skipped this pass.
- Session mechanism restart-invalidates-all-sessions tradeoff (see phase 5
  summary above) — a deliberate design decision, not a defect. Revisit only
  if restarts turn out to be more frequent than expected once this is in
  real use.
- ~~**New, from the second pass:** `categories::insert`/`update` and
  `documents::insert_label`/`update_label` in `core` have no server-side
  non-empty-name check~~ **Fixed** at the tail of
  `phase-5-web-remaining-sections` (commit `9e109f2`, landed just before
  this CLAUDE.md section was last updated, but never reflected here until
  now): `core` now rejects a blank/whitespace-only name for category,
  document_label, donor, and recipient_project alike.
- **New, from the second pass** (still open, not a bug — just a documented
  precedent): `crates/web/src/routes/outbound.rs`'s item picker needed
  several checkboxes sharing one `name="item_ids"`, and `axum::Form`'s
  `serde_urlencoded` backend can't deserialize repeated keys into a `Vec`
  (confirmed empirically — it errors rather than collecting). That route
  uses `axum::extract::RawForm` + `form_urlencoded::parse` directly instead
  of this crate's usual `#[derive(Deserialize)] + Form<T>` pattern, the
  only route that does. Documented inline; flagging here too in case a
  future multi-select field elsewhere in `web` needs the same approach and
  a reader goes looking for a precedent.

What the first pass's audit found *correct* and not to be "improved"
during the move: `db/queries/*` (parameterized, no business logic),
`model/*` (enum `label()`/`as_str()`/`is_inflow()` helpers are domain
vocabulary, correctly placed), `src/reports/{csv,pdf}.rs` (pure
renderers), and the CRUD-only views (`donors.rs`, `settings.rs`). The
second pass's reviews reused and confirmed all of these calls held for the
remaining seven sections too, with the three fixed 🔴-equivalent
exceptions described in the phase 5 summary above.

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

### Deploying the web service

Templates live in `deploy/`: `adm-sfa-web.service`, `adm-sfa-backup.service`
+ `adm-sfa-backup.timer`, `backup.sh`. Every `<PLACEHOLDER>` in the `.service`
files must be filled in for your actual machine before installing — none of
this has been applied anywhere; see the phase 6 note above for why.

**Design choice already made for you**: desktop's existing data directory
(its current default, `~/.local/share/adm-sfa` under whichever interactive
user runs it — confirm with that user's actual home dir, not assumed) stays
exactly where it is. The dedicated `adm-sfa` service user gets *group*
access to that same path rather than the data migrating to a system
location like `/var/lib/adm-sfa`. If that's wrong for your setup, the
`.service` files' `<DATA_DIR>`/`ReadWritePaths` placeholders are the only
thing that needs to change — nothing else in this section assumes one path
over the other.

1. **Create the service user and share data access via group, not by
   changing ownership** (changing ownership would break the interactive
   user's own read/write access to their existing data):
   ```sh
   sudo useradd --system --no-create-home --shell /usr/sbin/nologin adm-sfa
   sudo usermod -aG <interactive-user's-primary-group> adm-sfa
   chmod -R g+rwX ~<interactive-user>/.local/share/adm-sfa
   # new files/dirs created afterward need the setgid bit to keep inheriting
   # group-write, or the desktop app's own umask needs to leave group-write on:
   find ~<interactive-user>/.local/share/adm-sfa -type d -exec chmod g+s {} \;
   ```
2. **Build and install the binary**:
   ```sh
   cargo build --release -p web
   sudo install -m 755 target/release/adm-sfa-web /usr/local/bin/adm-sfa-web
   sudo install -m 755 deploy/backup.sh /usr/local/bin/adm-sfa-backup.sh
   ```
3. **Password file** — see "Setting `ADM_SFA_WEB_PASSWORD`" above for how to
   pick one; for the systemd deployment specifically:
   ```sh
   sudo mkdir -p /etc/adm-sfa
   sudo sh -c 'printf "ADM_SFA_WEB_PASSWORD=your-password-here\n" > /etc/adm-sfa/web-password.env'
   sudo chmod 600 /etc/adm-sfa/web-password.env
   sudo chown root:root /etc/adm-sfa/web-password.env
   ```
4. **Fill in the `<PLACEHOLDER>`s** in `deploy/adm-sfa-web.service` and
   `deploy/adm-sfa-backup.service` (data dir, LAN IP, backup staging dir,
   backup remote destination), then install the units:
   ```sh
   sudo cp deploy/adm-sfa-web.service deploy/adm-sfa-backup.service deploy/adm-sfa-backup.timer /etc/systemd/system/
   sudo systemctl daemon-reload
   sudo systemctl enable --now adm-sfa-web.service
   sudo systemctl enable --now adm-sfa-backup.timer
   ```
5. **Firewall — scope to the LAN subnet, not just the bind address.**
   Binding `ADM_SFA_WEB_BIND` to the LAN interface IP (not `0.0.0.0`) is one
   layer; the firewall rule is what actually restricts *which hosts* on
   that interface can reach it. Example using `ufw` (adapt for
   `nftables`/`iptables` if that's what the machine uses):
   ```sh
   sudo ufw allow from <LAN_SUBNET e.g. 192.168.1.0/24> to any port 8080 proto tcp
   ```
6. **Verify**: `sudo systemctl status adm-sfa-web` should show active
   (running); `curl http://<LAN_IP>:8080/login` from another machine on the
   subnet should get the login page; from outside the subnet it should time
   out or be refused.

`adm-sfa-backup.service`'s remote destination needs to be reachable
non-interactively overnight (SSH key in `adm-sfa`'s environment, or an
rsync-daemon target) — there's no prompt-for-credentials path in a
`Type=oneshot` unit triggered by a timer.

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

### Setting `ADM_SFA_WEB_PASSWORD`

`web` refuses to start without it (`crates/web/src/main.rs`) — there is no
default and no fallback. It's the single shared password for the whole app
(no per-user accounts, no DB storage, no hashing — see phase 5's summary
above), so treat it like any other shared secret: not on the command line
(shell history), not committed anywhere.

For a one-off dev run — POSIX `sh`/dash doesn't support bash's `read -s`, so
hide the input via `stty` directly instead (this is what `read -s` does
internally anyway, and it works in any shell):

```sh
stty -echo; printf 'Password: '; read ADM_SFA_WEB_PASSWORD; stty echo; printf '\n'
export ADM_SFA_WEB_PASSWORD
cargo run -p web
```

(bash/zsh users can use the shorter `read -s ADM_SFA_WEB_PASSWORD` if
preferred — just not portable to `sh`/dash.)

For a longer-running instance on the Linux machine, before phase 6 wires up
a systemd `EnvironmentFile=`, use a permission-restricted env file instead
of exporting it by hand each time:

```sh
mkdir -p ~/.config/adm-sfa
printf 'ADM_SFA_WEB_PASSWORD=your-password-here\n' > ~/.config/adm-sfa/web.env
chmod 600 ~/.config/adm-sfa/web.env         # world/group-readable defeats the point

set -a; . ~/.config/adm-sfa/web.env; set +a   # "." not "source" — POSIX sh doesn't have `source`
cargo run -p web
```

`ADM_SFA_WEB_BIND` (default `127.0.0.1:8080`, LAN-wide binding deferred to
phase 6) and `ADM_SFA_DATA_DIR` / `--data-dir` (same convention as
`desktop`) are the other two environment knobs `web` reads at startup.
