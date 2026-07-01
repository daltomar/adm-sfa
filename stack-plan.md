# adm-sfa bootstrap decisions

All decisions confirmed through planning session. Review before running any scaffold.

> **Revision note:** the initial draft of this plan hardcoded item categories and
> document labels instead of treating them as the first-class/config-driven
> entities the spec calls non-negotiable, and pinned several crates to
> versions that had since gone stale. Both are corrected below (see "Item
> category" and "Document label list", and the refreshed dependency table).

## Stack

| Concern | Choice | Key reason |
|---|---|---|
| Storage | `rusqlite` + `rusqlite_migration` | Sync, zero system deps (`bundled` feature), lightweight migration via `user_version` pragma |
| UI | `egui` / `eframe` | Pure Rust, no WebView/Node.js, built-in drag-and-drop, virtual-scroll tables via `egui_extras` |
| File dialogs | `rfd` | Native OS dialogs, used for both document attach and backup save-as |
| PDF reports | `typst-as-lib` + `typst-bake` | Template-driven, professional tables, bakes into binary (zero runtime deps) — **deferred from scaffold; add after spike** |
| eframe renderer | `glow` (OpenGL) for scaffold build | wgpu + naga + ash + vello together exceed available disk; switch to `default-features` once disk allows or wgpu is needed |
| CSV export | `csv` crate | Straightforward |
| Decimal arithmetic | `rust_decimal`, stored as TEXT in SQLite | Avoids IEEE-754 rounding; NUMERIC/REAL affinity silently corrupts financial data |
| Backup | `zip` + `walkdir` + `rfd` | ~50 lines; user picks save path, whole data dir zipped |
| Data directory | `~/.local/share/adm-sfa/` via `dirs` crate | Single folder, backup-able as one unit |
| Item categories | `category` table (id, name), FK from `inventory_item` | First-class entity per spec — new categories are a row insert, not a migration |
| Document labels | `document_label` table (id, name), seeded at migration time | Config-driven per spec — new labels are a row insert, not a source change |

`--data-dir <path>` CLI flag to override data directory (useful for development).

## Document label list

Config-driven via `document_label` table, not a source-code const. The UI's
label picker (`widgets/document_panel.rs`) populates its dropdown from
`SELECT name FROM document_label ORDER BY name`. Adding a new label is an
`INSERT` — no recompile, no schema migration.

Seeded at `001_initial.sql` migration time with the starting set:

```
"ad" | "chat" | "receipt" | "nota_fiscal" | "photo" | "other"
```

`document.label` stores the label name directly (TEXT, not an FK) so existing
document rows stay valid even if a label is later renamed or retired —
matches the soft-delete philosophy of never breaking historical records.

## Settings view (category & document_label management)

`ui/views/settings.rs` — simple list + add/rename UI, one section each for
`category` and `document_label`. No delete from this view for MVP: both are
referenced by historical rows (`inventory_item.category_id` is a hard FK;
`document.label` is a soft string copy), so removing entries safely needs
either a "retire" flag or a reassignment flow, which is more than a v1
settings screen needs. Rename is safe for both — `category` propagates
everywhere via the FK, and `document_label` renames only affect the dropdown
going forward since existing `document.label` values are already copied as
text.

If this gets deferred out of the initial scaffold, note it as an explicit
known gap rather than silently dropping it — the two tables exist by v1, and
there needs to be *some* way to add a category or label without a raw SQL
console, even if the UI polish comes later.

## Project layout

```
adm-sfa/
├── Cargo.toml
├── schema.sql                  # canonical schema, kept in sync with migrations/
├── migrations/001_initial.sql
├── templates/report.typ        # Typst PDF template
└── src/
    ├── main.rs                 # --data-dir, open DB, migrate, launch eframe
    ├── app.rs                  # App state + eframe::App + nav routing
    ├── db/
    │   ├── mod.rs              # open_db(), run_migrations(), Decimal ToSql/FromSql
    │   └── queries/            # donors, eur_ledger, brl_ledger, purchases,
    │                           # transfers, inventory, outbound, documents
    ├── model/                  # plain Rust structs/enums mirroring DB rows
    │   └── (donor, transaction, purchase, transfer, inventory, outbound,
    │        document, category, document_label)
    ├── ui/
    │   ├── sidebar.rs
    │   ├── views/              # dashboard, donors, eur_ledger, brl_ledger,
    │   │                       # purchases, transfers, inventory, outbound,
    │   │                       # reports, settings
    │   └── widgets/
    │       ├── document_panel.rs   # drag-drop zone, thumbnails, label picker
    │       └── amount_field.rs     # decimal text input with validation
    ├── reports/
    │   ├── pdf.rs              # typst-as-lib integration
    │   └── csv.rs
    ├── docs_fs.rs              # generate_filename(), copy_to_documents(), soft_delete()
    └── backup.rs               # backup_to_zip(data_dir, dest_path)
```

## First-draft schema (to become migrations/001_initial.sql)

```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE donor (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT    NOT NULL,
    contact_info TEXT,
    notes        TEXT
);

CREATE TABLE recipient_project (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    name         TEXT    NOT NULL,
    contact_info TEXT,
    location     TEXT,
    active       INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE annual_transfer (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    date                TEXT    NOT NULL,
    eur_amount_sent     TEXT    NOT NULL,
    exchange_rate       TEXT    NOT NULL,
    brl_amount_received TEXT    NOT NULL,
    notes               TEXT
);

CREATE TABLE purchase (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    date        TEXT    NOT NULL,
    currency    TEXT    NOT NULL CHECK (currency IN ('EUR', 'BRL')),
    cost        TEXT    NOT NULL,
    channel     TEXT    NOT NULL,
    seller_info TEXT
);

CREATE TABLE physical_donation (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    donor_id      INTEGER REFERENCES donor(id),
    date_received TEXT    NOT NULL,
    notes         TEXT
);

CREATE TABLE eur_transaction (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    date               TEXT    NOT NULL,
    type               TEXT    NOT NULL CHECK (type IN (
                           'donation_in', 'self_funding_in',
                           'purchase_out', 'transfer_to_brl_out')),
    amount             TEXT    NOT NULL,
    donor_id           INTEGER REFERENCES donor(id),
    note               TEXT,
    linked_purchase_id INTEGER REFERENCES purchase(id),
    linked_transfer_id INTEGER REFERENCES annual_transfer(id)
);

CREATE TABLE category (
    id   INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT    NOT NULL UNIQUE
);

CREATE TABLE inventory_item (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    name               TEXT    NOT NULL,
    category_id        INTEGER NOT NULL REFERENCES category(id),
    source_type        TEXT    NOT NULL CHECK (source_type IN ('donation', 'purchase')),
    source_donation_id INTEGER REFERENCES physical_donation(id),
    source_purchase_id INTEGER REFERENCES purchase(id),
    location           TEXT    NOT NULL CHECK (location IN ('germany', 'brazil')),
    status             TEXT    NOT NULL DEFAULT 'available'
                           CHECK (status IN ('available', 'reserved', 'donated')),
    notes              TEXT
);

CREATE TABLE outbound_event (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    date                 TEXT    NOT NULL,
    recipient_project_id INTEGER NOT NULL REFERENCES recipient_project(id),
    cash_amount_brl      TEXT,
    notes                TEXT
);

CREATE TABLE outbound_event_item (
    outbound_event_id INTEGER NOT NULL REFERENCES outbound_event(id),
    inventory_item_id INTEGER NOT NULL REFERENCES inventory_item(id),
    PRIMARY KEY (outbound_event_id, inventory_item_id)
);

-- brl_transaction is declared here (not alongside eur_transaction) because it
-- references outbound_event(id), which must exist first. Keeping table
-- declaration order forward-reference-free so this schema stays portable to
-- stricter engines/linters, even though SQLite itself resolves FK targets at
-- DML time and would tolerate the earlier ordering.
CREATE TABLE brl_transaction (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    date                     TEXT    NOT NULL,
    type                     TEXT    NOT NULL CHECK (type IN (
                                 'transfer_in', 'brazil_purchase_out', 'cash_gift_out')),
    amount                   TEXT    NOT NULL,
    linked_transfer_id       INTEGER REFERENCES annual_transfer(id),
    linked_purchase_id       INTEGER REFERENCES purchase(id),
    linked_outbound_event_id INTEGER REFERENCES outbound_event(id),
    note                     TEXT
);

CREATE TABLE document_label (
    id   INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT    NOT NULL UNIQUE
);

-- Polymorphic attachment: record_type IN ('purchase','item','transfer')
-- FK integrity enforced in application code, not SQL.
-- label stores the name (not an FK) so historical documents stay valid
-- even if a label is later renamed or retired from document_label.
CREATE TABLE document (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    filename    TEXT    NOT NULL UNIQUE,
    record_type TEXT    NOT NULL CHECK (record_type IN ('purchase', 'item', 'transfer')),
    record_id   INTEGER NOT NULL,
    label       TEXT    NOT NULL,
    deleted     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_eur_tx_date    ON eur_transaction(date);
CREATE INDEX idx_eur_tx_donor   ON eur_transaction(donor_id);
CREATE INDEX idx_brl_tx_date    ON brl_transaction(date);
CREATE INDEX idx_inv_cat        ON inventory_item(category_id);
CREATE INDEX idx_inv_status_loc ON inventory_item(status, location);
CREATE INDEX idx_outbound_proj  ON outbound_event(recipient_project_id);
CREATE INDEX idx_doc_record     ON document(record_type, record_id);

-- Seed data (part of 001_initial.sql, not a separate migration)
INSERT INTO category (name) VALUES
    ('complete'), ('deck'), ('trucks'), ('wheels'), ('bearings'),
    ('pads'), ('helmet'), ('shoes'), ('other');

INSERT INTO document_label (name) VALUES
    ('ad'), ('chat'), ('receipt'), ('nota_fiscal'), ('photo'), ('other');
```

## Cargo dependencies (to add at scaffold time)

Versions below confirmed against crates.io as of 2026-07-01.

```toml
[dependencies]
# glow backend only — avoids wgpu/naga/ash/vello for the scaffold build.
# Switch to default-features (removes this override) once disk allows or wgpu is needed.
eframe             = { version = "0.35", default-features = false, features = ["glow", "x11", "wayland", "default_fonts"] }
egui_extras        = { version = "0.35", features = ["image"] }
rfd                = "0.17"
rusqlite           = { version = "0.40", features = ["bundled"] }
rusqlite_migration = "2.6"
rust_decimal       = "1"
rust_decimal_macros = "1"
# typst-as-lib and typst-bake deferred — spike them in isolation before adding here.
# Original planned entries (add after spike passes):
#   typst-as-lib = "0.16"   # API explicitly marked unstable — pin exact patch version
#   typst-bake   = "0.1"    # early-development crate — see risk note below
serde              = { version = "1", features = ["derive"] }
serde_json         = "1"
csv                = "1"
zip                = "8"      # major jump from originally-considered v2 — verify API before scaffolding
walkdir            = "2"
dirs               = "6"
```

**Risk note — PDF pipeline:** `typst-as-lib` documents its own API as "not
really stable," and `typst-bake` describes itself as "very new" / "early
development." Both are core to the "PDF report renders a readable table"
checklist item. Before investing UI time in `reports/pdf.rs`, do a standalone
spike: bake one static Typst template into a throwaway binary and confirm it
compiles and renders on your target OS. If `typst-bake`'s compile-time baking
turns out to be too rough, fall back to `typst-as-lib` alone (loading the
template from `templates/report.typ` at runtime instead of baking it in) —
this drops the "zero runtime deps" property but keeps everything else intact.

**Risk note — `zip` v2 → v8:** six major versions is enough that the crate's
API has almost certainly changed shape (error types, builder patterns, etc.).
Since `backup.rs` is a small, isolated ~50-line module, this is a cheap spike
to derisk early rather than discovering breakage mid-implementation.

## Verification checklist (after scaffold)

- [ ] `cargo build` succeeds, no warnings
- [ ] Window opens; sidebar shows all sections
- [ ] Create a donor → appears in list
- [ ] Attach file to a purchase → file in `documents/` with auto-generated name; `document` row in DB
- [ ] Soft-delete document → file in `documents/_deleted/`; `deleted = 1` in DB
- [ ] PDF report renders a readable table
- [ ] "Backup now" produces a `.zip` with DB + `documents/` tree
- [ ] Reopen app → data persists; migrations don't re-run
