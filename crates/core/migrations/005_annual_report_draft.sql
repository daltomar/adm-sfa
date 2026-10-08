-- Stores the user-authored narrative sections for each fiscal year's
-- annual statutory reports (Tätigkeitsbericht + Protokoll). The financial
-- figures themselves are always recomputed from the live ledger data; only
-- the prose that cannot be derived from the DB is persisted here.
CREATE TABLE annual_report_draft (
    year               INTEGER PRIMARY KEY,
    member_count       INTEGER NOT NULL DEFAULT 0,
    meeting_date       TEXT    NOT NULL DEFAULT '',
    meeting_time_from  TEXT    NOT NULL DEFAULT '',
    meeting_time_to    TEXT    NOT NULL DEFAULT '',
    -- Tätigkeitsbericht narrative sections (user-authored prose)
    tb_activities      TEXT    NOT NULL DEFAULT '',
    tb_continuous      TEXT    NOT NULL DEFAULT '',
    tb_outlook         TEXT    NOT NULL DEFAULT '',
    -- Protokoll narrative sections (user-authored prose)
    pk_decisions       TEXT    NOT NULL DEFAULT '',
    pk_activities_next_year TEXT NOT NULL DEFAULT '',
    updated_at         TEXT    NOT NULL DEFAULT (datetime('now'))
);
