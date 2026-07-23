ALTER TABLE purchase ADD COLUMN status TEXT NOT NULL DEFAULT 'bought'
    CHECK (status IN ('negotiating', 'bought'));
