# archive.md

Historical implementation notes moved out of CLAUDE.md. These sections document
decisions, trade-offs, and review findings for features that are fully shipped and
no longer need active attention. Useful as background if a related area is revisited.

---

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

## Purchase creation with documents in one step (implemented)

Branch `purchase-create-with-documents`. Creating a purchase and attaching
documents to it used to be two separate steps on both front-ends: Save the
purchase fields, land on the now-existing record, *then* attach a document
via a separate action, one at a time. Now the "new purchase" flow on both
`desktop` and `web` lets the user stage one or more documents (label +
file) alongside the purchase fields and files everything in a single Save.

- New `crates/core/src/service.rs` primitives: `PendingDocument<'a>`
  (borrowed path + label), `AttachmentOutcome` (per-file result),
  `attach_documents` (loops the existing single-document `attach_document`
  over a batch, folding each success's filename into the de-dupe list
  before the next attempt so two same-label same-date files in one batch
  don't collide), and `create_purchase_with_documents` (inserts the
  purchase via the existing `create_purchase`, then calls
  `attach_documents`). The composed function deliberately returns `Ok`
  even when some staged documents failed to attach — the purchase row is
  real and saved either way, and there's no shared transaction wrapping
  the insert and the file operations (unwinding a saved purchase because
  one unrelated upload failed is not the desired behavior, and file copies
  aren't transactional regardless). `Err` is reserved for the purchase
  itself failing to save, in which case no attachment is attempted at all.
  `CreatedPurchase` is `#[must_use]` so a caller can't silently discard
  attachment failures by ignoring the whole return value — it doesn't
  catch discarding just `.attachments` while keeping `.id`, which still
  relies on the doc comment plus both current call sites actually
  checking it, per the code review.
- `web`: `POST /purchases` (`crates/web/src/routes/purchases.rs`) changed
  from urlencoded `Form<PurchaseForm>` to `Multipart`, hand-parsing
  purchase fields plus repeated `doc_label`/`doc_file` pairs (paired
  positionally, not by index — `Multipart` streams fields regardless of
  name repetition, so unlike `outbound.rs`'s `RawForm` workaround for
  repeated keys, nothing special is needed here). The original filename
  from each `doc_file` part is tracked alongside its generated temp path
  and label, specifically so a partial-failure status message can name
  the file the user actually picked rather than the meaningless generated
  temp filename `AttachmentOutcome::source_name` would otherwise show
  (`core` derives `source_name` from whatever path it's handed, and has no
  concept of "the caller wrote this to a temp file first") — caught during
  manual verification, not by the code review or the automated tests, and
  fixed with a zipped-by-index lookup plus a regression test. On full
  success (including the zero-attachments case), redirects to `/purchases`
  — changed from the old `/purchases/{id}/edit` target, per the owner's
  request. On a partial attach failure, renders the edit page (the
  purchase now has a real id) with a per-file success/failure list, via a
  `purchase_form_response` helper generalized from the old
  `purchase_form_error_response`. `purchases/form.html` gained a
  "Documents? Yes/No" toggle (conditional `enctype="multipart/form-data"`,
  since the edit page's `update()` route must keep reading a plain
  urlencoded body) revealing repeatable label+file rows via vanilla JS —
  modeled on the only prior client-side toggle in this codebase
  (`eur_ledger/form.html`), extended with row-cloning and, critically,
  explicit `disabled` toggling alongside `display:none` (a hidden but
  still-enabled control is still submitted).
- `desktop`: `crates/desktop/src/ui/views/purchases.rs`'s existing
  single-attachment picker UI (drag-and-drop, browse-by-path, screenshot
  capture, the pending-file confirm/cancel group) was extracted into a
  shared `show_attachment_picker`, used by both the existing Editing-mode
  `show_documents` and a new Adding-mode `show_staged_documents`. A new
  `staged_docs: Vec<PendingAttachment>` field holds documents confirmed
  via the picker before Save; the Save button's `Mode::Adding` branch now
  calls `create_purchase_with_documents` with all staged docs, then walks
  the per-file outcomes in *reverse* index order (so `remove(i)` doesn't
  shift later indices) to prune successes — deleting their temp file if
  screenshot-sourced — while leaving failures in `staged_docs` with their
  error set. `show_documents` (now Editing-mode) pulls the first
  still-failed staged doc into the normal `pending_doc` slot at the top of
  its render, before the picker's own drag-and-drop pickup runs, so a
  failed attachment gets retried through the exact same Attach/Cancel flow
  as any other document — no separate retry UI. `discard_staged_docs`
  (mirroring `discard_pending_doc`) is called at all four existing
  form-reset sites so an abandoned "new purchase" flow can't leak staged
  screenshot temp files.
- Both front-ends' new-purchase flow now supports staging *multiple*
  documents before the first Save (not just one), per the owner's
  explicit choice — they expect to often attach 2-3 documents (e.g. a
  chat screenshot plus an invoice) when logging a purchase, and didn't
  want to bounce to the edit page immediately after Save just to attach a
  second one. Attaching a document to an *already-saved* purchase (a
  third, fourth, ... document, or one added in a later session) is
  unchanged — that still goes through the edit page's existing
  one-at-a-time attach mini-form on both front-ends.
- 6 new tests in `crates/core/src/service.rs` (batch success, in-batch
  filename dedup, partial failure without rollback, zero attachments,
  missing source file, purchase-insert failure files nothing) and 6 new
  route tests in `crates/web/src/routes/purchases.rs` (two-document
  success redirecting to the list, zero-document success, an empty file
  row being silently skipped, one unknown label producing a partial
  failure with the original filenames shown, an invalid date rerendering
  the new form with nothing created, unauthenticated access redirecting
  to login) — the latter needed `test_support::multipart_request_with_parts`,
  a generalization of the existing single-label/single-file
  `multipart_request` to an arbitrary ordered list of parts, with the old
  function reimplemented on top of it unchanged in signature so its five
  existing callers kept passing untouched.
- Reviewed by `rust-code-reviewer`: no 🔴 findings. Two 🟡s surfaced, not
  fixed as part of this branch:
  - `create_purchase_with_documents`'s "partial failure is `Ok`" contract
    relies on callers actually inspecting `.attachments` — `#[must_use]`
    on `CreatedPurchase` (added after the review) catches a caller that
    discards the whole return value, but not one that reads `.id` and
    never touches `.attachments`. Both current call sites do check it;
    revisit if a third call site is ever added.
  - `create`'s multipart-parsing loop (`while let Ok(Some(field)) = ...`)
    silently stops on a mid-stream read error (e.g. a client disconnecting
    partway through an upload) rather than surfacing it distinctly from
    "the user left a field blank" — pre-existing debt shared by the
    analogous loops in `attach_document` here and in `transfers.rs`/
    `inventory.rs`, not introduced by this branch, and not fixed as part
    of it.

## EUR Ledger: linking Purchase-/Transfer-typed rows to their record (implemented)

Branch `eur-ledger-linked-record-link`. Bug report: a "Purchase"-typed EUR
Ledger row had no way to open the purchase it was auto-created from. Same
gap existed for "Transfer"-typed rows. Both are now linked on both
front-ends.

- **Web**: `EurLedgerRow.editable: bool` (and its now-unused `id` field)
  replaced with `link_href: Option<String>`
  (`crates/web/src/templates.rs`), computed per-row by a new `link_href()`
  in `crates/web/src/routes/eur_ledger.rs` — a manual entry
  (Donation/Self-funding) links to its own `/eur-ledger/{id}/edit` as
  before; a Purchase-typed row links to `/purchases/{linked_purchase_id}
  /edit`; a Transfer-typed row links to
  `/transfers/{linked_transfer_id}/edit`. No core/query change needed —
  `EurTxRow.linked_purchase_id`/`linked_transfer_id` were already
  populated by the existing query, just unused by the route until now.
  `crates/web/templates/eur_ledger/list.html` renders the link whenever
  `link_href` is `Some`. Two new tests in
  `crates/web/src/routes/eur_ledger.rs` cover both linked types.
- **Desktop**: clicking a Purchase-/Transfer-typed row already opened a
  read-only detail panel (`EurLedgerView::show_linked_info`,
  `Mode::ViewingLinked`) — it just had no way to jump to the actual record.
  This needed new cross-section navigation plumbing, since nothing in the
  app previously let one view tell `App` to switch `Section` and
  pre-select a record in a different view:
  - `crates/desktop/src/ui/views/eur_ledger.rs`: new `pub enum
    LedgerNavTarget { Purchase(i64), Transfer(i64) }`, a `nav_request:
    Option<LedgerNavTarget>` field, and `take_nav_request()`.
    `show_linked_info` (now `&mut self`) renders an "Open in
    Purchases"/"Open in Transfers" button when the row's
    `linked_purchase_id`/`linked_transfer_id` is `Some`, setting
    `nav_request` on click.
  - `crates/desktop/src/ui/views/{purchases,transfers}.rs`: new `pub fn
    select_for_edit(&mut self, db: &Connection, id: i64)` on each view —
    fetches the record directly via the existing `qry::get` (doesn't
    assume the view's already-loaded list contains it, since the caller
    may jump here without ever having visited that section this session)
    and enters `Mode::Editing(id)` with the right draft. Deliberately not
    factored into a shared helper with each view's existing list-row-click
    handler — small, two call sites, different data source (a single
    fetch vs. an already-loaded list element).
  - `crates/desktop/src/app.rs`: after the `CentralPanel` block, checks
    `eur_ledger_view.take_nav_request()` and, if `Some`, switches
    `self.section` and calls the target view's `select_for_edit`. One
    frame lands late (the request is set *during* the same frame's
    `CentralPanel` closure, so the section switch only takes visible
    effect next frame) — harmless at 60fps, and confirmed neither view's
    `invalidate()` touches `mode`, so the just-set `Editing(id)` survives
    the next frame's section-change invalidation.
- Reviewed by `rust-code-reviewer`: no 🔴 findings. One 🟡 fixed before
  commit: `select_for_edit`'s fetch-fails path (unreachable in practice —
  the button only exists when the linked FK is live — but not impossible,
  e.g. the linked record was deleted through some other path) used to
  return silently, landing the user on an unexplained list view with no
  error after `app.rs` had already switched sections. Now sets
  `self.error` (a new `purchases.error.linked_record_not_found`/
  `transfers.error.linked_record_not_found` key) and resets to
  `Mode::List` instead.

## Donors: creating one from the main Donors page lands on the list (implemented)

Branch `web-donors-create-redirect`. Bug report: on `web`, `POST /donors`
with no `return_to` (the normal "Donors page → + Add donor" flow)
redirected to the new donor's own `/donors/{id}/edit` instead of back to
the Donors list. Desktop was already correct — `ui/views/donors.rs`'s
Save-while-adding handler already returns to `Mode::List` on success, never
opening a per-donor edit view — so this was a web-only fix.

- `crates/web/src/routes/donors.rs`'s `create()`: the fallback branch of
  `if safe_return_to(&return_to) { .. } else { .. }` (taken both for "no
  `return_to` submitted" and for a rejected/unsafe one — these were already
  handled identically before this change, sharing one branch, just with a
  different shared destination) now redirects to `/donors` instead of
  `/donors/{id}/edit`. The `return_to`-present-and-safe branch (used by
  every "+ New donor" link from EUR Ledger/Inventory/Outbound, which
  appends `?donor_id={id}` and redirects back to the linking page) is
  unchanged. `update()`'s post-edit redirect (still `/donors/{id}/edit`,
  matching desktop's own Save-while-*editing* behavior, which does stay on
  the edit view) is untouched — this only ever affected the create path.
- Three existing security-regression tests for the open-redirect guard
  (`create_ignores_an_unsafe_return_to`,
  `create_ignores_a_protocol_relative_return_to`,
  `create_ignores_a_backslash_return_to`) had their expected `location`
  updated from `/donors/1/edit` to `/donors` — their actual assertion (the
  attacker-supplied value never reaches `Redirect::to`) is unaffected by
  which safe destination the fallback picks. New test
  `create_without_a_return_to_redirects_to_the_donors_list` covers the
  actual bug report.
- Reviewed by `rust-code-reviewer`: no 🔴 or 🟡 findings.

## Web Inventory: Source UX, create-with-documents, "+ New donation" round trip (implemented)

Branch `web-inventory-source-type-ux` (3 commits). Three related fixes to
the web New/Edit Item form, each reviewed independently by
`rust-code-reviewer` with no 🔴 findings:

- **Same-line radios + mandatory Source.** Location/Status/Source radios
  now render on one line each via the existing `.radio-group` class. The
  New Item form's Source radios (Donation/Purchase) start unselected and
  `required`, with a JS toggle showing only the matching dropdown — mirrors
  `web-eur-ledger-type-ux`'s Typ UX. Closes the same bug class that fix
  closed: `create()`/`update()` used to silently default a missing/invalid
  `source_type` to Donation instead of rejecting it; both now reject
  authoritatively via `SourceType::from_str`. The Edit form keeps
  pre-selecting an item's persisted `source_type` (unlike eur-ledger's
  `tx_type`, this stays editable after creation) — only New starts blank.
  Desktop untouched: egui's `radio_value` already only shows one sub-panel
  per selected enum value, so it never had the dual-visible-dropdown bug.
- **Create-with-documents.** `/inventory/new` gained the same "Attach
  documents to this item?" toggle as `/purchases/new` — stage one or more
  label+file pairs, filed in the same submission as the item. New
  `core::service::create_item`/`CreatedItem`/`create_item_with_documents`,
  mirroring `create_purchase_with_documents`/`create_transfer_with_documents`
  (`""`/`None` for date resolution, since `InventoryItemDraft` has no date
  field of its own — matches the existing single-document `attach_document`
  call sites for items). Full success redirects to `/inventory`; a partial
  attach failure re-renders the edit page with a per-file status list.
  `update()` and the edit page's one-at-a-time attach form are unchanged.
- **"+ New donation" round trip.** Creating a donation via the Source
  dropdown's "+ New donation" link used to strand the user on
  `/inventory/donations` after saving. Now mirrors `/eur-ledger/new`'s "+
  New donor" flow exactly: `safe_return_to` (the open-redirect guard) moved
  from `donors.rs` to `routes/mod.rs` as `pub(crate)`, now shared instead of
  duplicated; `inventory::new_form` reads back `name`/`category_id`/
  `location`/`status`/`notes` plus a `donation_id` query param (validated
  against the real donations list — `core` has no dedicated
  `get_donation(conn, id)` the way `donors_qry::get` exists for donors, so
  this checks membership in `list_donations` instead; logged as a 🟢
  completeness nit, not fixed, since a second call site hasn't shown up
  yet); `donations`/`create_donation` gained `return_to` handling identical
  in shape to `donors.rs`'s. `inventory/donations.html`'s own always-visible
  "back to New Item" hint link also honors `return_to` when present (a
  small scope addition beyond the literal bug report, not just the
  post-save redirect), so navigating away without saving a donation still
  preserves the in-progress item fields.

Fixed (commit `f90accc`): `inventory/donations.html`'s own "+ New donor"
link now gets the same JS-populated `return_to` treatment
`inventory/form.html`'s "+ New donation" link already had — it carries
`date_received`/`notes` plus its own incoming `return_to` forward via a
`data-outer-return-to` attribute (HTML-attribute-context interpolation,
same pattern as the page's existing hidden `return_to` input), read back
through `.dataset` (which the browser HTML-entity-decodes automatically).
`donations()`'s `DonationsQuery` gained `date_received`/`notes`/`donor_id`
fields to read the round trip back on the way in, and a `donor_options()`
helper (mirroring `eur_ledger.rs`'s) validates and preselects the donor.
`donors.rs::create` needed no changes, confirmed generic as predicted.
Verified end-to-end against a live running server (full item → donation →
donor → back chain, encoding traced hop by hop) in addition to two new
route tests. Reviewed by `rust-code-reviewer`: no findings.

## Web: on-screen amounts ignoring `ui_locale` (implemented)

Found while adding test coverage to `brl_ledger.rs` (below): most of
`web`'s on-screen money/rate formatting was locale-independent despite the
app's T1/T3 i18n design. `core::format::amount()`/`number()` read the
*ambient* `rust_i18n::locale()`, but `crates/web/src/main.rs` deliberately
never calls `rust_i18n::set_locale` (it's a single process-wide global,
unsafe to set per-request across concurrent users on different locales) —
so every amount/rate rendered in the process's fallback locale ("en")
regardless of the resolved `ui_locale`, while translated *text* was
unaffected (that already went through an explicit-locale `t()` filter).
`reports.rs` was the sole exception, already using the explicit-locale
`format::amount_in(value, locale)` throughout — that became the pattern to
generalize. Verified live: with `ui_locale=de`, a BRL purchase rendered
`-R$ 9,876.54` (English) before the fix, `-R$ 9.876,54` (German) after.

- Added `format::number_in(value, decimals, locale)` alongside the existing
  `amount_in()`; expanded `amount()`'s doc comment to explain the
  ambient-vs-explicit hazard specifically for `web` (safe for `desktop`,
  which does call `set_locale` on locale change — this is a `web`-only
  bug).
- Switched every ambient `format::amount()`/`format::number()` call site in
  `crates/web` to the explicit-locale variant, threading the
  already-resolved `locale` through: `brl_ledger.rs`, `eur_ledger.rs`
  (balance + row amount), `outbound.rs` (`event_summary`'s cash suffix),
  `transfers.rs` (`brl_preview` and `list`'s eur/brl/rate columns),
  `inventory.rs` (`purchase_label`, gained a `locale` parameter),
  `purchases.rs` (`list`'s `cost_display`) — 11 call sites across 6 files.
  `desktop` is untouched, correctly — it's supposed to keep using the
  ambient form.
- One regression test per fixed file (6 total), each setting `ui_locale`
  to German via the real `settings` query and asserting the rendered HTML
  contains the German-formatted string and not the English one.
- Commit `0a635b6`. Reviewed by `rust-code-reviewer`: one 🟡 (a
  `cargo fmt` nit) fixed before commit; a note about `cargo clippy
  --all-targets` surfacing `await_holding_lock` in test code (pre-existing,
  not introduced by this change, and invisible to the project's own
  documented `cargo clippy --workspace -- -D warnings` command since that
  never compiles `#[cfg(test)]` modules) left as a documented, non-blocking
  observation, not fixed.

## Post-save success/error banner (implemented)

Branch `save-status-banner`. Neither front-end confirmed a successful save
before this — desktop silently flipped back to list/stayed on the form; web
did a bare `Redirect::to(...)` with nothing attached. A banner now appears
after the main record create/update Save action in both front-ends: 8
sections — purchases, donors, eur_ledger, transfers, inventory, outbound,
and settings' category/document_label CRUD (6 actions) — persists until the
user navigates away (no auto-dismiss timer). `brl_ledger` is out of scope
on both front-ends (no create/update anywhere), and so are secondary
actions — mark-bought, drop-negotiating-purchase, attach/remove document,
locale change, manual backup, and every inline "+ New X" create reached via
a `return_to` round trip (e.g. "+ New donor" from another section's form) —
these keep their existing behavior unchanged. Pure presentation, no `core`
involvement: `core`'s `Result<_, _>` returns already are the domain-level
success/error signal: both front-ends just carry that signal one render
further than before (web: across a redirect; desktop: across the frame
where `mode` changes or stays).

- **Web**: new `crates/web/src/flash.rs` — a short-lived (30s), read-once,
  signed cookie (`adm_sfa_flash`, mirroring `auth.rs`'s session-cookie
  security attributes) set via `set_flash` right before a save handler's
  redirect, and read-and-cleared via `take_flash` by the GET handler at
  the redirect target (the PRG pattern — no session store beyond the one
  auth cookie existed to carry this otherwise). `FlashKind` is two-armed
  (`Success`/`Error`) for symmetry with desktop's equivalent, but no
  in-scope call site ever constructs `Error` — every failing create/update
  already re-renders its own form inline with the pre-existing `.error`
  div (no redirect on failure, so no cross-request problem there). New
  `templates::Flash` struct threaded into the 12 template structs that are
  genuine redirect targets (list *and* form templates where `update`
  redirects to the edit page rather than the list, e.g.
  `PurchaseFormTemplate`/`TransferFormTemplate`/`InventoryFormTemplate`/
  `OutboundFormTemplate`); `eur_ledger/form.html` deliberately has no flash
  field since both its create and update redirect to the *list*, never
  back to itself. New `{% block flash %}{% endblock %}` in `layout.html`
  ahead of `{% block content %}`, overridden identically by all 12
  templates. New `common.status.save_success`/`save_failed` i18n keys
  (`locales/{en,de,pt-BR}.yml`).
- **Desktop**: reintroduces `crates/desktop/src/ui/widgets/` (CLAUDE.md's
  note on the deleted `document_panel.rs` stub said to add it back only if
  a second shared widget materialized — this is that widget):
  `status_banner::show(ui, &Option<Result<String,String>>)`, a one-line
  no-op-if-`None` render generalizing the ad hoc pattern `settings.rs`
  already used locally for `screenshot_status`/`backup_status`. A new
  `save_status` field on the 7 in-scope views (donors, purchases,
  eur_ledger, transfers, inventory, outbound, settings — `brl_ledger` has
  no insert/update anywhere) is set on each Save button's `Ok` arm and
  cleared at every existing `error = None` transition site (Cancel,
  +Add/New, row-select-to-edit, `select_for_edit`) plus in each view's
  `invalidate()` — the latter is what makes "gone on navigate-away-and-
  back" correct, since switching `Section` calls `invalidate()` on
  re-entry. Rendered once, at the top of each view's `show()`, before the
  list/form split — necessary because which `Mode` a view lands in after a
  successful Save differs by section (donors/eur_ledger's create returns
  to `Mode::List`; purchases/transfers/inventory/outbound's create stays in
  `Mode::Editing(new_id)` to allow immediate document attachment; every
  section's update never changes mode at all).
- A real bug was found only by manual live-server testing, not by the
  first automated-test pass: `take_flash`'s original removal cookie
  (`jar.remove(Cookie::from(FLASH_COOKIE))`) had no explicit `path("/")`.
  A `Set-Cookie` with no path defaults to the *request URI's directory*
  (RFC 6265), not `/` — so consuming the flash on a nested route like
  `/purchases/1/edit` emitted a deletion scoped to `/purchases/1`, which a
  real browser (correctly) leaves the original `Path=/` cookie alone for,
  so it survived and reappeared on the next unrelated `/purchases` list
  load a moment later. Fixed by building the removal cookie as
  `Cookie::build(FLASH_COOKIE).path("/")`.
- 7 new tests in `crates/web/src/flash.rs` (round-trip, tampered/absent
  cookie, the path-scoping regression above) and one dedicated flash-cookie
  round-trip route test per representative section (`donors.rs`,
  `purchases.rs`'s update-lands-on-edit-page case, `settings.rs`).
- Reviewed by `rust-code-reviewer`: no 🔴 findings. One 🟡 (settings.rs's
  6 CRUD actions had no flash-specific test, unlike every other touched
  section) fixed before commit by adding
  `create_category_redirects_and_the_index_page_shows_the_flash`.

## Web locale picker (implemented)

Branch `web-locale-picker`. The web Settings page now has a locale picker — `POST /settings/locale` validates the submitted code against `core::format::LOCALES` and writes the shared `ui_locale` `app_setting` key, the same key desktop's picker writes. The web already read this key per-request via `i18n::resolve_locale()` and passed it explicitly to every `t!()` call (safe for concurrent requests), so the write path was the only missing piece. No new i18n keys for the picker itself — `settings.locale.heading`, `settings.locale.hint`, and `settings.locale.field.language` already existed in all three locale files from when desktop's picker was added. One new key added: `settings.locale.error.unknown` (rejected locale code — should never appear via the normal form's `<select>`, which only offers the three valid codes).

- `SettingsTemplate` gained a `locales: Vec<(String, String)>` field (code + endonym pairs from `LOCALES`), populated by `settings_template()`. The `<select>` pre-selects whichever code matches the current `locale` field (`code.as_str() == locale.as_str()` — Askama comparison that avoids the `&String == String` type mismatch a plain `==` would produce in generated code).
- `web.settings.hint` updated in all three locale files: the locale picker is no longer desktop-only, so the old "locale picker … desktop-only" wording was a falsehood; now names only the screenshot command and manual backup as desktop-only.
- 3 new tests: `set_locale_redirects_and_the_setting_is_saved`, `set_locale_with_an_unknown_code_rerenders_with_an_error`, `set_locale_without_a_session_cookie_redirects_to_login`. Reviewed by `rust-code-reviewer`: no 🔴 findings; one 🟡 (stale `web.settings.hint`) fixed before commit; one 🟡 (unauthenticated-access test for the new route) fixed before commit; two 🟢 style notes (missing flash round-trip test, `as_str()` verbosity) left as-is.
- Desktop behaviour note: desktop reads `rust_i18n::locale()` (its in-memory process-wide global set at startup, not the DB) — a locale change made via the web is not reflected in a running desktop session until the user opens desktop's own Settings picker or restarts the app. Accepted; matches how a desktop change wasn't visible in web until the next web request before this change.

## Web front-end: mobile-responsive layout (implemented)

Branch `web-mobile-responsive` (PR #31, 2026-08-24). The header nav (9 links
in one flex row) and roughly a dozen data tables had no responsive handling,
so they overflowed narrow phone viewports.

- `layout.html`'s header now collapses into a CSS-only hamburger menu
  (checkbox toggle, no JS) below 768 px.
- All list and form tables wrapped in a `.table-scroll` container for
  horizontal scroll on narrow screens.
- A mobile media query drops the 400 px input cap, stacks action buttons
  full-width, and tightens page padding.
- Settings' hardcoded 220 px inputs gained `max-width: 100%` so they shrink
  on narrow screens.
- One 🔴 finding from `rust-code-reviewer` fixed before commit: a global
  `box-sizing: border-box` reset was scoped to `.button` only to avoid
  shrinking desktop's main content area by ~48 px.
- No schema changes. One new i18n key per locale (hamburger accessible label).
  Pure CSS/HTML, no JS.

---

## Workspace restructure and web front-end — phase history

Goal: extract the domain layer into a shared crate so a web front-end can
be added alongside the existing desktop app, both running on the same
internal-LAN machine against one database.

### Phases (all complete)

1. **Workspace split.** Mechanical move only — no logic or signature
   changes beyond what visibility requires. Fold in the configurable data
   root, and enable WAL mode, here.
2. **Invariants into `core`.** Push down the cross-row rules currently
   enforced only in view code. Highest priority: the outbound item status
   guard, which is a real data-integrity gap today and an exploitable one
   once an HTTP client can call it.
3. **Reports aggregation into `core`.** Extract `build_donor_rows`, the
   EUR/BRL summary folds, `build_audit_entries`, and the free functions
   (`in_range`, the `*_tx_description` helpers, `donor_or_anonymous`) out
   of `ui/views/reports.rs`. Unify the three `compute_balance` copies into
   one. **Done** — see `crates/core/src/reporting.rs`; 14 focused unit
   tests on the extracted functions.
4. **Service layer.** Operation-shaped functions in `core`
   (`create_purchase`, `mark_purchase_bought`, `donate_items`,
   `attach_document`, …). Desktop views call these instead of reaching
   into `db::queries` directly. **Done** — see `crates/core/src/service.rs`
   plus `docs_fs::remove_document` (a real ordering fix: soft-deleting a
   document now moves the file *before* marking the DB row deleted) and a
   new `outbound::require_gift` guard.
5. **Web crate.** axum + server-rendered templates over the service layer.
   Multipart upload replacing drag-and-drop; file serving for
   `documents/`; single shared password + session cookie. **Done, full
   section parity** — see `crates/web/` (new binary `adm-sfa-web`, Askama
   0.12 templates under `crates/web/templates/`). Auth: single shared
   password from `ADM_SFA_WEB_PASSWORD` (constant-time compare via
   `subtle`), signed session cookie (`axum-extra`'s `SignedCookieJar`,
   `HttpOnly` + `SameSite=Strict`, no per-user identity). `AppState.db` is
   a single `Connection` behind a plain `Mutex` (not a pool), matching
   `desktop`'s single-`Connection` shape; `AppState::conn()` recovers from
   mutex poisoning rather than panicking the whole server on one bad request.

   Three real security gaps fixed during phase 5's review — all in the same
   category: a business rule previously enforced only in `desktop`'s UI,
   now reachable by an untrusted HTTP client:
   - `eur_ledger::insert`/`update` and `transfers::insert`/`update` had no
     amount-positivity check in `core`. Fixed in `core::db::queries::{eur_ledger,
     transfers}`, with regression tests. The `transfers` fix also caught a
     `Decimal` multiplication overflow panic — switched to `checked_mul`.
   - `inventory::check_purchase_source` never checked a purchase's
     `negotiating`/`bought` status. Fixed in
     `core::db::queries::inventory::check_purchase_source`, with
     regression tests.
   - `crates/web/src/routes/reports.rs::export_csv` spliced the `tab`
     query param into a temp file path with only an emptiness check, not
     the same `TABS` allowlist the on-screen route already used — a path
     traversal. Fixed by validating `tab` the same way both routes now.

6. **Deployment.** systemd unit, own `adm-sfa` user, hardening, nightly
   backup. **Done** — artifacts under `deploy/`; see "Deploying the web
   service" in CLAUDE.md for the install sequence. Templates only —
   nothing has been applied to the real machine from a session.

### Resolved backlog (phases 2–5)

All items below were found during phase reviews and subsequently fixed.
Listed here for audit trail only.

**Phase 2:**
- `link_items` had no item-status check → fixed, `available`-only guard
  in transaction.
- `multiple_items_unset_conflict` only checked in view → fixed in
  `purchases::update`.
- `purchase_source_conflict` duplicated in view → fixed in
  `inventory::purchase_source_conflict`.
- Source-type radio switch left stale FK on the other type → fixed with
  `.changed()` clear.
- `compute_balance` defined three times → unified in `reporting::compute_balance`.
- Donated inventory item had no locked fields → fixed on
  `backlog-donated-item-field-locking` (PR #16): full lock, notes-only editable.

**Phase 3:**
- `brl_summary` missing starting-balance test (mirror of EUR's) → added.
- `build_audit_entries` transfer-branch untested → test added.

**Phase 4:**
- `drop_negotiating_purchase` test only covered happy path → failure-stops-loop
  test added.
- `docs_fs::generate_filename` didn't consult `_deleted/` filenames → fixed via
  `merge_deleted_filenames`.

**Phase 5 backlog (PRs #12–#16):**
- Document label free-text with no allowlist → fixed (PR #12).
- `web` had zero automated tests → full coverage added (PRs #14, #16 + subsequent).
- "Re-fetch list + `.find()`" pattern for single-row lookups → `get(conn, id)`
  added per entity (PR #15).
- `parse_data_dir` duplicated across binaries → moved to `adm_sfa_core::config`
  (desktop gained `ADM_SFA_DATA_DIR` support as a side effect).
- `/logout` was a `GET`, session cookie had no `Max-Age` → fixed (PR #13).
- Web templates had hardcoded English strings → `t!()` wired everywhere
  (commit `aa9ec4c`).
- Transfers form BRL preview only appeared after save, not live → fixed
  (PR #23, commit `8a2c74e`).
- Blank-name inserts for category/document_label/donor/recipient_project →
  `core` now rejects whitespace-only names (commit `9e109f2`).
- Session restart invalidates all sessions — deliberate design decision,
  not a defect. `require_auth` checks signature, not issued-at; 8-hour
  `Max-Age` is client-enforced only.
