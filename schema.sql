-- Canonical schema — keep in sync with migrations/.
-- Pragmas are set at connection time in db::open_db(), not here.

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
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    date            TEXT    NOT NULL,
    currency        TEXT    NOT NULL CHECK (currency IN ('EUR', 'BRL')),
    cost            TEXT    NOT NULL,
    channel         TEXT    NOT NULL,
    seller_info     TEXT,
    multiple_items  INTEGER NOT NULL DEFAULT 0,
    status          TEXT    NOT NULL DEFAULT 'bought' CHECK (status IN ('negotiating', 'bought'))
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

-- Declared after outbound_event to keep all REFERENCES forward-reference-free.
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

-- Polymorphic attachment: record_type IN ('purchase','item','transfer').
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
