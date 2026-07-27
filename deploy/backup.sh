#!/bin/sh
# Nightly backup: a consistent SQLite snapshot via the online backup API
# (safe to run against a live WAL-mode DB, unlike copying adm-sfa.db
# directly) plus an rsync mirror of documents/, both sent off-machine.
#
# This is separate from the desktop app's manual "Backup now" button
# (crate::backup::backup_to_zip) — that one is a point-in-time zip a human
# triggers from Settings; this one is the unattended nightly job CLAUDE.md's
# phase 6 calls for. Run via adm-sfa-backup.timer, not by hand.
set -eu

DATA_DIR="${ADM_SFA_DATA_DIR:?set ADM_SFA_DATA_DIR before running this script}"
BACKUP_DIR="${ADM_SFA_BACKUP_DIR:?set ADM_SFA_BACKUP_DIR (local staging dir for DB snapshots) before running this script}"
REMOTE="${ADM_SFA_BACKUP_REMOTE:?set ADM_SFA_BACKUP_REMOTE (rsync destination, e.g. user@host:/path/) before running this script}"
KEEP_DAYS="${ADM_SFA_BACKUP_KEEP_DAYS:-14}"

mkdir -p "$BACKUP_DIR"

TIMESTAMP="$(date +%Y%m%d-%H%M%S)"
SNAPSHOT="$BACKUP_DIR/adm-sfa-$TIMESTAMP.db"

sqlite3 "$DATA_DIR/adm-sfa.db" ".backup '$SNAPSHOT'"

rsync -a --delete "$DATA_DIR/documents/" "$REMOTE/documents/"
rsync -a "$SNAPSHOT" "$REMOTE/db-backups/"

# Prune local snapshots older than KEEP_DAYS — the remote side keeps every
# copy rsync has ever sent unless pruned separately there; local staging
# doesn't need to.
find "$BACKUP_DIR" -maxdepth 1 -name 'adm-sfa-*.db' -mtime "+$KEEP_DAYS" -delete
