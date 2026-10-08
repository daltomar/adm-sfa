# Migration rehearsal procedure

Run this before deploying any build that adds a new migration. The goal is
to confirm that the migration succeeds on a copy of the real data and that
the data looks correct afterward. **Claude Code must never run these steps
against the live data directory — always use the scratch copy.**

## Step 1 — Take a point-in-time copy of the live database

Close the desktop app first (no open WAL transactions), then:

```sh
SCRATCH=/tmp/adm-sfa-rehearsal-$(date +%Y%m%d)
mkdir -p "$SCRATCH"
sqlite3 ~/.local/share/adm-sfa/adm-sfa.db ".backup '$SCRATCH/adm-sfa.db'"
cp -a ~/.local/share/adm-sfa/documents "$SCRATCH/documents"
```

Verify the copy is intact:

```sh
sqlite3 "$SCRATCH/adm-sfa.db" "PRAGMA integrity_check;"
# must print: ok
```

## Step 2 — Record baseline counts and a spot-check report

```sh
sqlite3 "$SCRATCH/adm-sfa.db" "
  SELECT 'purchase',      COUNT(*) FROM purchase      UNION ALL
  SELECT 'inventory_item',COUNT(*) FROM inventory_item UNION ALL
  SELECT 'eur_transaction',COUNT(*) FROM eur_transaction UNION ALL
  SELECT 'brl_transaction',COUNT(*) FROM brl_transaction UNION ALL
  SELECT 'donor',         COUNT(*) FROM donor          UNION ALL
  SELECT 'document',      COUNT(*) FROM document;
"
```

Save this output — you will compare it after the migration.

## Step 3 — Run the new binary against the scratch copy

```sh
cargo build --release -p web
cargo run --release -p web -- --data-dir "$SCRATCH"
```

Or for desktop:

```sh
cargo run --release -p desktop -- --data-dir "$SCRATCH"
```

The binary will automatically apply the pending migration and (for web) write
a pre-migration snapshot to `$SCRATCH/backups/`. Check the startup logs: the
binary must reach "listening on …" (web) or the main window (desktop) without
panicking or printing a migration error.

## Step 4 — Verify counts and integrity after migration

```sh
sqlite3 "$SCRATCH/adm-sfa.db" "PRAGMA integrity_check;"
# must print: ok

sqlite3 "$SCRATCH/adm-sfa.db" "
  SELECT 'purchase',      COUNT(*) FROM purchase      UNION ALL
  SELECT 'inventory_item',COUNT(*) FROM inventory_item UNION ALL
  SELECT 'eur_transaction',COUNT(*) FROM eur_transaction UNION ALL
  SELECT 'brl_transaction',COUNT(*) FROM brl_transaction UNION ALL
  SELECT 'donor',         COUNT(*) FROM donor          UNION ALL
  SELECT 'document',      COUNT(*) FROM document;
"
```

Compare with the Step 2 baseline. Row counts must match (or increase if
the migration adds rows) — a decrease means data was lost during migration.

## Step 5 — Spot-check a report export

Open the web UI (from Step 3) or the desktop app pointed at `$SCRATCH` and
generate a PDF or CSV report. Confirm the totals and line items look correct.

## Step 6 — Deploy only after rehearsal passes

If all of Steps 1–5 passed:

1. Stop the running web service (`sudo systemctl stop adm-sfa-web`).
2. Install the new binary (`sudo install -m 755 target/release/adm-sfa-web /usr/local/bin/`).
3. Start the service (`sudo systemctl start adm-sfa-web`).
4. Check logs immediately (`journalctl -u adm-sfa-web -n 30`).
5. Confirm the startup pre-migration snapshot was written to
   `~/.local/share/adm-sfa/backups/`.

Rebuild and redeploy both binaries in the same maintenance window —
the two binaries share one database and running mismatched versions will
cause the older one to refuse to start (S0.3 guard).
