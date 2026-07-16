# Skateboard donation project — application specification

Desktop application (Rust) for tracking dual-currency cash flow, physical inventory, and outbound donations between Germany and Brazil.

## 1. Overview

This application manages a small charitable project that collects physical donations of skateboard-related material, purchases additional material using donated and self-funded money, and distributes everything to recipient projects. The project operates across two countries and two currencies: donations and purchases in Germany are in EUR, and once a year funds are transferred to Brazil to cover local purchases and occasional small cash gifts to recipient projects, in BRL.

The application must support: full traceability of every euro and every real from source to destination, photographic and documentary proof for every purchase, a flexible inventory of physical items with status and location tracking, and ad-hoc reporting suitable for personal record-keeping, donor transparency, and authorities/tax purposes.

## 2. Core principles

- Single user, single machine — no multi-user sync required.
- File-on-disk storage for all documents and photos, referenced by path from the database — keeps the database small and the whole project backup-able as one folder.
- Nothing is ever silently deleted. Document removal is a soft-delete (moved to an archive folder), preserving the audit trail — the sole exception is dropping a still-negotiating purchase, which has no ledger or inventory footprint (see §3.6).
- Currency conversion happens at exactly one point: the annual EUR→BRL transfer, where the rate is entered manually. EUR and BRL are otherwise independent ledgers.
- Money amounts are typed as free text and parsed leniently: either a comma or a period works as the decimal separator (not both together — an amount like "1.234,56" is rejected rather than guessed at), and a leading currency symbol (R$, €, $) is stripped if present — matching how amounts are naturally typed across this app's EUR/BRL, German/Brazilian context. Text that still doesn't parse as a number is rejected with a visible error, not a silently-disabled form.
- No hardcoded purchase channels — Kleinanzeigen is the common case but the model supports any channel via a generic structured note field.
- Recipient projects, donors, and item categories are first-class, reusable entities, not free text, so they can be reported on.

## 3. Data model

Types are suggestions for a SQLite-backed Rust implementation; exact types can be refined during implementation.

### 3.1 Donor

A reusable entity so individual contributions can be tracked over time, even though donations are not mandatory to attribute to a named donor.

| Field | Type | Notes |
|---|---|---|
| id | integer (PK) | Auto-increment |
| name | text | Required |
| contact_info | text | Optional — email, phone, etc. |
| notes | text | Optional free text |

### 3.2 Physical donation

A direct donation of physical material. Always creates one or more inventory items.

| Field | Type | Notes |
|---|---|---|
| id | integer (PK) | Auto-increment |
| donor_id | integer (FK → Donor) | Optional — anonymous donations allowed |
| date_received | date | Required |
| notes | text | Optional |

### 3.3 EUR cash transaction

A single ledger covering all EUR inflows and outflows. The running balance is the sum of all entries.

| Field | Type | Notes |
|---|---|---|
| id | integer (PK) | Auto-increment |
| date | date | Required |
| type | enum | donation_in \| self_funding_in \| purchase_out \| transfer_to_brl_out |
| amount | decimal (EUR) | Always positive; sign implied by type |
| donor_id | integer (FK → Donor) | Set only when type = donation_in |
| note | text | Optional, e.g. self-funding justification |
| linked_purchase_id | integer (FK) | Set when type = purchase_out — covers Kleinanzeigen and any other EUR-side channel |
| linked_transfer_id | integer (FK) | Set when type = transfer_to_brl_out |

### 3.4 Annual transfer (EUR → BRL)

Represents money moved from the EUR ledger to the BRL ledger once a year. The exchange rate is entered manually and is not linked to any external rate feed.

| Field | Type | Notes |
|---|---|---|
| id | integer (PK) | Auto-increment |
| date | date | Required |
| eur_amount_sent | decimal (EUR) | Required — deducts from EUR balance |
| exchange_rate | decimal | Manually entered, EUR→BRL |
| brl_amount_received | decimal (BRL) | = eur_amount_sent × exchange_rate, adds to BRL balance |
| notes | text | Optional |
| documents | Document[] | Transfer receipt, etc. — see Section 4 |

### 3.5 BRL cash transaction

A separate ledger, independent of the EUR ledger except for inflow from the annual transfer.

| Field | Type | Notes |
|---|---|---|
| id | integer (PK) | Auto-increment |
| date | date | Required |
| type | enum | transfer_in \| brazil_purchase_out \| cash_gift_out |
| amount | decimal (BRL) | Always positive; sign implied by type |
| linked_transfer_id | integer (FK) | Set when type = transfer_in |
| linked_purchase_id | integer (FK) | Set when type = brazil_purchase_out |
| linked_outbound_event_id | integer (FK) | Set when type = cash_gift_out |
| note | text | Optional |

### 3.6 Purchase

A generic purchase event in either currency/channel. Always deducts from the relevant cash ledger and adds one or more items to inventory.

| Field | Type | Notes |
|---|---|---|
| id | integer (PK) | Auto-increment |
| date | date | Required |
| currency | enum | EUR \| BRL |
| cost | decimal | In the listed currency |
| channel | text | Free text, e.g. "Kleinanzeigen", "local supplier" — not a fixed enum |
| seller_info | text | Generic structured note: name, address, contact, listing details, etc. Same field for all channels. |
| status | text | `negotiating` \| `bought`. Default `bought`. A negotiating purchase is recorded but writes no ledger row and creates no inventory items until it transitions to `bought`. `bought` is terminal. |
| multiple_items | boolean | When false (default), the purchase links to exactly one inventory item; when true, multiple items may share it (lot purchases). |
| documents | Document[] | Ad screenshots, chat screenshots, receipts, nota fiscal — see Section 4 |

**Negotiation status.** A purchase may be entered as `negotiating` to capture an in-progress deal (e.g. an active Kleinanzeigen chat) without committing it to the ledger. No EUR/BRL cash transaction is written and no inventory item may be created against it while negotiating. Confirming the deal is a single edit to `bought`, which triggers the ledger write. Reverting `bought → negotiating` is out of scope (it would require reversing a ledger entry). Dropping a negotiating purchase hard-deletes the *purchase row only* — safe only because a negotiating purchase has never touched the ledger or inventory; this is the single deliberate exception to §2's soft-delete principle, permitted precisely because no auditable financial or inventory state exists yet. Any documents already attached to the purchase (ad/chat screenshots) are **not** hard-deleted or orphaned alongside it: they go through the normal document soft-delete path (§4.5) first, same as removing a document from any other record.

### 3.7 Recipient project

The organizations the project donates to. New ones can be added freely.

| Field | Type | Notes |
|---|---|---|
| id | integer (PK) | Auto-increment |
| name | text | Required |
| contact_info | text | Optional |
| location | text | Optional |
| active | boolean | Allows retiring a project without deleting history |

### 3.8 Inventory item

Every physical item, whether donated directly or purchased.

| Field | Type | Notes |
|---|---|---|
| id | integer (PK) | Auto-increment |
| name | text | Required, e.g. "Complete skateboard", "Helmet, size M" |
| category_id | integer (FK → Category) | First-class entity, not a fixed enum — see note below |
| source_type | enum | donation \| purchase |
| source_donation_id | integer (FK) | Set when source_type = donation |
| source_purchase_id | integer (FK) | Set when source_type = purchase |
| location | enum | germany \| brazil |
| status | enum | available \| reserved \| donated — reserved is optional, never mandatory |
| documents | Document[] | Photos — see Section 4 |
| notes | text | Optional |

### 3.8a Category

A reusable entity per §2 ("item categories are first-class, reusable entities"), not a fixed enum — new categories can be added without a schema change, matching the treatment of donors and recipient projects.

| Field | Type | Notes |
|---|---|---|
| id | integer (PK) | Auto-increment |
| name | text | Required, unique |

Starting set (seeded at setup, not hardcoded thereafter): `complete`, `deck`, `trucks`, `wheels`, `bearings`, `pads`, `helmet`, `shoes`, `other`.

### 3.9 Outbound donation event

A single act of giving to a recipient project — items, BRL cash, or both. Cash gifts always draw from the BRL ledger.

| Field | Type | Notes |
|---|---|---|
| id | integer (PK) | Auto-increment |
| date | date | Required |
| recipient_project_id | integer (FK → Recipient project) | Required |
| cash_amount_brl | decimal (BRL) | Optional — 0 or null if items-only |
| notes | text | Optional |

### 3.10 Outbound donation item (join table)

Links inventory items to the outbound event they were given away in, and sets their status to donated.

| Field | Type | Notes |
|---|---|---|
| outbound_event_id | integer (FK) | — |
| inventory_item_id | integer (FK) | — |

## 4. Document and photo storage

### 4.1 Storage location

All files (photos, PDFs, screenshots) are stored on disk in a single flat folder, `documents/`, next to the database file. No per-record subfolders. The database stores only the filename (and implicitly the record it belongs to, via the filename structure and a documents join table).

### 4.2 Filename structure

Filenames are generated entirely by the application — the user never types a filename. Pattern:

```
{date}_{record-type}-{id}_{label}{-n}.{ext}
```

- **date** — the record's date (or upload date), format YYYY-MM-DD
- **record-type** — `purchase` \| `item` \| `transfer`
- **id** — the record's database ID
- **label** — selected from a dropdown when the file is attached: `ad`, `chat`, `receipt`, `nota_fiscal`, `photo`, `other`. The list is config-driven, not hardcoded, so new labels can be added later without a schema change.
- **-n** — auto-appended only when a label is already used on the same record (e.g. a second chat screenshot), starting at `-2`

Example: a Kleinanzeigen purchase (id 42) with an ad screenshot and two chat screenshots produces:

```
2026-06-30_purchase-42_ad.jpg
2026-06-30_purchase-42_chat.jpg
2026-06-30_purchase-42_chat-2.jpg
```

Filenames are locale-independent: none of `date`, `record-type`, or `label` is ever a UI-language-translated or display-formatted value — the filename generated for a given record is the same no matter which UI language was active when the file was attached (see §6.1).

### 4.3 Upload flow

The user drags a file onto a purchase, item, or transfer record. A small dialog shows the filename already filled in (date, record type, and ID are pre-populated and not editable) with only the label left to choose from a dropdown. No free typing is required.

### 4.4 Multiple documents per record

Purchases, inventory items, and transfers can each have any number of attached documents, shown as a thumbnail/file list on the record's detail view. Documents can be added at any time after the record is created.

### 4.5 Deletion

Deleting a document does not erase the file. It is moved from `documents/` to `documents/_deleted/` (filename unchanged) and unlinked from the record. This preserves the audit trail against accidental deletion; a separate, deliberate cleanup outside the application is the only way to permanently remove a file.

## 5. Reporting

Reports must serve three audiences with different needs: personal record-keeping, donor transparency (showing individual contributions), and authorities/tax purposes (requiring rigor and traceability). Given this, the application provides one flexible, filterable view rather than several fixed templates.

### 5.1 Filtering

- Date range (required support)
- Recipient project (required support)
- Both filters can be combined or used independently

### 5.2 Output formats

- On-screen — filterable tables as the primary, default view
- PDF export — clean, printable, suitable for sharing with donors or authorities
- CSV export — for further analysis or import into spreadsheets

Every report is generated in an explicit target language, independent of the active UI language (§6.3). CSV export additionally always uses a fixed German-style number/delimiter format regardless of the active UI language (§6.4).

### 5.3 Required report content

- Donor breakdown — contributions per individual donor over a date range (not just aggregate totals), to support donor transparency
- EUR ledger summary — donations in, self-funding in, purchases out (any channel), transfers out, running balance
- BRL ledger summary — transfer in, Brazil purchases out, cash gifts out, running balance
- Inventory summary — items by category, status, and location
- Outbound summary — items and cash given per recipient project over a date range
- Full audit trail — every transaction and outbound event in a date range, each with links/references to its attached documents, suitable as a standalone record for authorities

### 5.4 Open for implementation phase

Exact report layouts and any additional breakdowns (e.g. cost per category, year-over-year comparison) can be refined once the data model is implemented and real data is available to test against.

## 5.5 Dashboard (suggested — not yet implemented)

The Dashboard section is currently an empty placeholder. Suggested content for a future iteration, all derivable from already-loaded data with no new queries:

- **EUR balance** and **BRL balance** — the running totals already computed by the ledger views.
- **Inventory snapshot** — total item count broken down by status (available / reserved / donated).
- **Recent activity** — the last few transactions across both ledgers (date, type, amount).
- **Outbound this year** — total items donated and cash gifted in the current calendar year.

None of these require new schema or entities. Implementation can reuse the existing `list()` queries from the ledger, inventory, and outbound modules and aggregate in Rust, exactly as the Reports section does.

## 6. Localisation

The desktop UI supports three languages: English (`en`, the source and fallback language), German (`de`), and Brazilian Portuguese (`pt-BR`) — not European Portuguese, since the recipient projects are Brazilian and the vocabulary differs.

### 6.1 Scope

In scope: all UI chrome (labels, buttons, menus, table headers, dialogs, validation/error messages), locale-aware date/number/currency *display* formatting, and a persisted language selection that can be changed live, without restarting the application.

Out of scope, deliberately:

- Translating user-entered entity data (donor names, recipient project names, item category names, document labels) — each is entered once by the operator and displayed verbatim in every language, per §2's treatment of these as first-class reusable entities rather than translatable content.
- Translating DB-stored enum/status values (`purchase.status`, `inventory_item.status`, `inventory_item.location`, transaction types) — these remain English identifiers in the database and are mapped to a translated display string only at the point of rendering.
- Translating generated document filenames (§4.2) — filenames are, and remain, locale-independent.
- CSV report export formatting — see §6.4, which is fixed regardless of the active UI language.

### 6.2 Language setting

The active UI language is a single persisted setting (`en` \| `de` \| `pt-BR`, default `en`), stored using the same generic settings mechanism already used for other single-value configuration (e.g. the screenshot capture command) — no new data-model entity is introduced for it.

### 6.3 Report language is independent of UI language

Every report-generation entry point (on-screen, PDF, CSV — §5.2) takes an explicit target-language argument. It is never read implicitly from the active UI language setting. The UI language may be offered as the *default* choice in the report/export dialog, but the operator can generate, for example, a German-language report while the UI itself is set to English — the two are different questions ("what language is the report in" vs. "what language is the operator using the app in") and must stay answerable independently. A report that needs a fixed, non-translatable terminology set (e.g. legally-shaped official-language terms) may pin its own language outside the general translation catalogue; any such report specifies this for itself if and when it is scoped.

### 6.4 CSV export format is fixed, not language-dependent

CSV export (§5.2) always uses a German-style number and delimiter format — `;` field delimiter, `,` decimal separator, `.` thousands separator (e.g. `1.000,00`) — regardless of the active UI language. This is a deliberate exception to the general locale system: CSV output has one real-world consumer profile (German/Brazilian spreadsheet software, which both expect this format, and for which a comma decimal separator would make a comma field delimiter unusable), so there is no need for it to vary.

### 6.5 Formatting is separate from translation

Locale affects *presentation*, never stored data or computation:

- Monetary values are stored and computed exactly as described in §2, regardless of UI language.
- Amount *input* parsing (§2's comma-or-period leniency) is independent of the active UI language — a `de`-language user and an `en`-language user can each type either decimal-separator style; input leniency and display formatting are separate concerns.
- The currency symbol shown for an amount (€ or R$) is a property of which ledger the amount belongs to, not of the active UI language — a EUR amount always shows `€`, in every language.
- Date and number *display* formatting follows the active UI language (e.g. a date renders as `2026-07-16` in English, `16.07.2026` in German, `16/07/2026` in Portuguese); this affects on-screen and PDF display only, not CSV (§6.4) or stored values.

### 6.6 Fallback

If a display string has no translation in the active language, English is shown rather than a blank or placeholder value, and this never causes an error.

## 7. Open decisions for the implementation phase

> **Status:** resolved — see `stack-plan.md` for the chosen stack and schema.
> This section is left as-is for historical context on what was intentionally
> deferred out of the functional spec; it does not need to track the current
> tech choices.

The following were intentionally left for the Rust stack selection step, not part of this functional specification:

- Storage layer: `rusqlite` vs. `sqlx` for SQLite access
- UI framework: `egui` vs. `Tauri` vs. `iced`
- PDF generation library for report export
- Backup mechanism — manual "backup now" button is required functionally; the underlying implementation (copy folder, zip, cloud sync) is a technical decision
