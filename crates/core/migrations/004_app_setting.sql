-- Generic key-value app config, for settings that (unlike category /
-- document_label) aren't a list of user-managed entities — just single
-- scalar values. First consumer: the screenshot capture command.
CREATE TABLE app_setting (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
