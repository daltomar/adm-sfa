# CLAUDE.md

Project memory and working instructions for Claude Code in this repository.

## What this project is

A Rust application for a small charitable project that tracks physical
skateboard-related donations, dual-currency (EUR/BRL) cash flow, purchases, and
outbound donations to recipient projects in Brazil.

**Two front-ends, one domain core.** The original egui desktop app remains the
primary interface. A web front-end runs alongside it so a second, occasional user
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
  (`stack-plan.md` is not committed to the repo — it is `.gitignore`d by
  design; the decisions it documents are already fully built and described
  in this file and in `SPEC.md`)
  (`rusqlite` + `rusqlite_migration`), UI framework (`egui`/`eframe`), PDF
  library (`typst-as-lib` + `typst-bake`, with a fallback noted if the bake
  step proves too unstable), backup mechanism (`zip` + `walkdir`), and the
  first-draft `schema.sql`. Section 6 of `SPEC.md` is left unchanged as
  historical context on what was deferred — `stack-plan.md` is the current
  source of truth for tech choices.
- **Implementation: complete.** All SPEC.md sections implemented on both
  front-ends: Donors, EUR Ledger, BRL Ledger, Purchases (including the
  `multiple_items` flag and negotiation `status`), Transfers, Inventory,
  Outbound, Jahresbericht, Reports (on-screen + CSV export + PDF export via
  `typst-as-lib`), Settings (category + document label CRUD, screenshot
  capture command, locale picker). `web` has full section parity with
  `desktop`. Workspace restructure (all 6 phases) is done.
- **Backup:** the manual "backup now" button required by `SPEC.md §2` is
  wired up — `SettingsView::show_backup_panel` calls `crate::backup::backup_to_zip`.
- **Dashboard:** currently an empty placeholder. Suggested content is
  documented in `SPEC.md §5.5` — not yet prioritised.
- **Deployment templates:** `deploy/` — see "Deploying the web service" below.
  Not yet installed on the real machine (requires system user, firewall, etc.).
  Tag `v1.0-desktop` marks the pre-restructure state.

## Open backlog

Items confirmed open. See `archive.md` for the full resolved-backlog history.

- **Web date picker format** — `<input type="date">` displays in the browser's
  own locale (mm/dd/yyyy), owner wants dd/mm/yyyy. Firefox honors the page's
  `lang` attribute (already set); Chrome/Edge ignore it entirely. Two options,
  not yet decided: do nothing (fixes Firefox only), or replace with a
  vanilla-JS dd/mm/yyyy widget (touches all seven date-input templates). Owner
  asked to shelve for now.

- **Drag-and-drop document attach (desktop) — undiagnosed.** May not work on
  the target OS/desktop environment. Code is untouched by the restructure
  (phases 1–4 never touched `dropped_files`/`hovered_files` handling), so if
  broken it was pre-existing, not a regression. The "Attach file" button (manual
  path entry) is a working fallback. To diagnose: reproduce on the real machine,
  check whether `hovered_files` fires at all vs. only the drop event, check
  `eframe`/`winit` 0.35's changelog for known Wayland/X11 drag-and-drop issues.

- **`outbound.rs` edit path calls `db::queries::outbound::update` directly**
  (not through a `service::*` wrapper) — minor asymmetry against the phase 4
  goal of "views call service functions, not `db::queries`". Both the create
  and edit paths run the identical `require_gift` guard, so this is cosmetic
  for now. Decide before adding a third outbound call site whether a
  `service::update_donation` wrapper should exist.

## Workspace layout

```
adm-sfa/
  Cargo.toml          # workspace
  crates/
    core/             # model/, db/, schema.sql, migrations/, docs_fs, backup, config
    reports/          # csv + typst rendering (pure renderers, no aggregation)
    desktop/          # existing egui UI + screenshot.rs
    web/              # axum + server-rendered templates
```

`desktop` and `web` both depend on `core` and `reports`. Neither knows the
other exists.

**`core`'s actual package name is `adm_sfa_core`, not the literal string
`core`.** Naming a workspace crate `core` shadows Rust's own sysroot `core`
crate in the extern prelude of every crate that depends on it — confirmed
during phase 1 with a live repro (`use core::mem;` inside `desktop` silently
resolved to the local crate instead of `::core` once a dependency named
`core` existed, rather than failing loudly). The directory stays
`crates/core/`; only the `[package].name` / `use adm_sfa_core::...` differ.
Depend on it as `adm_sfa_core = { path = "../core" }`, not
`core = { path = "../core" }`.

## Platform differences between front-ends

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
- **Manual backup** stays desktop-only — no equivalent in `web`.

## Deploying the web service

Templates live in `deploy/`: `adm-sfa-web.service`, `adm-sfa-backup.service`
+ `adm-sfa-backup.timer`, `backup.sh`. Every `<PLACEHOLDER>` in the `.service`
files must be filled in for your actual machine before installing — none of
this has been applied anywhere.

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
3. **Password file** — see "Setting `ADM_SFA_WEB_PASSWORD`" below for how to
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
5. When adding a feature, implement it in `core` first, then wire up *both*
   front-ends — or state explicitly that it's platform-specific and why.
   Silently shipping a feature to only one front-end is the failure mode to
   avoid.
6. When adding a feature after the workspace restructure, do not reach into
   `db::queries` directly from UI crates — go through `core::service::*`
   (or add a new service function if one doesn't exist yet).

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
- **Two users, one machine, no sync.** The web front-end gets a single shared
  password with a session cookie — not per-user accounts, roles, or permissions.
  Both binaries open the same SQLite file directly; SQLite runs in WAL mode.
  Do not add sync, replication, per-user identity, or an ORM/connection-pool
  layer to "solve" concurrency — WAL plus rare overlapping use is the whole
  design. Stale reads in a long-open desktop session are accepted, not worked
  around.
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

- **Module layout** (paths are post-restructure):
  in `crates/core`: `db/` for `rusqlite` access and query modules per
  entity group, `model/` for plain structs mirroring DB rows (including
  `category` and `document_label` as first-class models, not enums),
  `docs_fs.rs` for filename generation, the shared document-filing helper,
  and soft-delete, `backup.rs` for the zip-based backup, `service.rs` for
  operation-shaped functions that views call, `reporting.rs` for aggregation
  logic.
  In `crates/reports`: PDF/CSV rendering only — no aggregation.
  In `crates/desktop`: `ui/views/` for one file per section (including
  `settings.rs` for category/label/screenshot-command management) and
  `screenshot.rs` for OS screenshot-tool invocation. `ui/widgets/` contains
  `status_banner.rs` (the shared post-save banner widget).
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
- **Multi-checkbox form fields in `web`**: `axum::Form`'s `serde_urlencoded`
  backend cannot deserialize repeated keys into a `Vec` (errors rather than
  collecting). Use `axum::extract::RawForm` + `form_urlencoded::parse`
  directly, as `outbound.rs`'s item picker already does — the only route
  that currently needs this pattern.
- **`format::amount()` / `format::number()` in `web`**: these read the
  ambient `rust_i18n::locale()`, which `web` never sets per-request (unsafe
  across concurrent users). Always use the explicit-locale variants
  `format::amount_in(value, locale)` / `format::number_in(value, decimals,
  locale)` in `web` route handlers and templates. `desktop` correctly uses
  the ambient form.

## Code review

After completing any feature or fix and before committing, delegate the
changed code to the `rust-code-reviewer` subagent. Address 🔴 findings
before the commit; surface 🟡/🟢 for me to decide.

## Useful commands

```sh
cargo build --workspace                      # compile everything
cargo run -p desktop                         # run the egui app
cargo run -p desktop -- --data-dir /tmp/test # alternate data dir
cargo run -p web                             # run the web server
cargo test --workspace                       # run all tests across crates
cargo clippy --workspace --all-targets -- -D warnings  # lint everything (includes tests)
cargo fmt --all                              # format all crates
```

### Setting `ADM_SFA_WEB_PASSWORD`

`web` refuses to start without it (`crates/web/src/main.rs`) — there is no
default and no fallback. It's the single shared password for the whole app
(no per-user accounts, no DB storage, no hashing), so treat it like any
other shared secret: not on the command line (shell history), not committed
anywhere.

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

For a longer-running instance on the Linux machine, before the systemd
`EnvironmentFile=` is wired up, use a permission-restricted env file:

```sh
mkdir -p ~/.config/adm-sfa
printf 'ADM_SFA_WEB_PASSWORD=your-password-here\n' > ~/.config/adm-sfa/web.env
chmod 600 ~/.config/adm-sfa/web.env         # world/group-readable defeats the point

set -a; . ~/.config/adm-sfa/web.env; set +a   # "." not "source" — POSIX sh doesn't have `source`
cargo run -p web
```

`ADM_SFA_WEB_BIND` (default `127.0.0.1:8080`) and `ADM_SFA_DATA_DIR` /
`--data-dir` (same convention as `desktop`) are the other two environment
knobs `web` reads at startup.
