# Deployment Guide: adm-sfa web — test-production with real data

## Context

Deploying `adm-sfa-web` alongside the already-running desktop app, both pointing at the same SQLite database and `documents/` folder on one Linux machine. The primary risk is **the web service silently opening the wrong directory and creating a fresh empty database**, or **permissions being misconfigured so uploads corrupt or fail mid-write**. Everything else is recoverable; those two are not obvious until damage is done.

---

## Step 0 — Back up first (non-negotiable)

Before touching the server, take a manual point-in-time copy of the entire data directory. Do this **while the desktop app is closed** (no open WAL transactions):

```sh
# Close the desktop app first, then:
cp -a ~/.local/share/adm-sfa ~/adm-sfa-backup-$(date +%Y%m%d)
```

If the desktop app was open, the WAL files (`adm-sfa.db-wal`, `adm-sfa.db-shm`) must be copied alongside `adm-sfa.db` — **never copy just the `.db` file while the app is open**, or you get a partial snapshot.

Verify the backup: `sqlite3 ~/adm-sfa-backup-YYYYMMDD/adm-sfa.db "PRAGMA integrity_check;"` should print `ok`.

---

## Step 1 — Find your exact data directory path

The desktop app resolves its data dir via `--data-dir`, `ADM_SFA_DATA_DIR`, or the OS default (`~/.local/share/adm-sfa`). Find which one it's actually using:

```sh
ls -la ~/.local/share/adm-sfa/          # check default location
ls -la ~/.local/share/adm-sfa/adm-sfa.db   # must exist
ls -la ~/.local/share/adm-sfa/documents/   # must exist
```

**This exact path is `<DATA_DIR>` for everything that follows.** If it's somewhere else (custom `--data-dir`), find it now. Getting this wrong in the service file is the #1 silent data-loss risk — the web service would start cleanly with a fresh empty DB and you'd never know until you logged in and saw no data.

---

## Step 2 — Build the binary

On the server machine (or cross-compile and copy):

```sh
cargo build --release -p web
sudo install -m 755 target/release/adm-sfa-web /usr/local/bin/adm-sfa-web
sudo install -m 755 deploy/backup.sh /usr/local/bin/adm-sfa-backup.sh
```

---

## Step 3 — Create the service user

```sh
sudo useradd --system --no-create-home --shell /usr/sbin/nologin adm-sfa
```

Then give it group read/write access to the data directory **without changing ownership** (which would break the desktop app's own access):

```sh
# Replace <you> with the interactive username that owns the data dir
sudo usermod -aG <you> adm-sfa          # add adm-sfa to your primary group
chmod -R g+rwX ~/.local/share/adm-sfa   # group read+write on all files/dirs
find ~/.local/share/adm-sfa -type d -exec chmod g+s {} \;  # setgid: new files inherit group
```

The `setgid` bit is critical: without it, documents created by the desktop (running as you) won't be group-writable, so the web service could fail to soft-delete or rename them later.

**Verify:** `sudo -u adm-sfa ls ~/.local/share/adm-sfa/` should succeed and show `adm-sfa.db` and `documents/`.

---

## Step 4 — Create the password file

```sh
sudo mkdir -p /etc/adm-sfa
sudo sh -c 'printf "ADM_SFA_WEB_PASSWORD=your-chosen-password\n" > /etc/adm-sfa/web-password.env'
sudo chmod 600 /etc/adm-sfa/web-password.env
sudo chown root:root /etc/adm-sfa/web-password.env
```

---

## Step 5 — Fill in the service file placeholders

Edit `deploy/adm-sfa-web.service`. The placeholders to replace:

| Placeholder | Replace with |
|---|---|
| `<DATA_DIR>` | The exact path from Step 1, e.g. `/home/danilo/.local/share/adm-sfa` |
| `<LAN_IP>:8080` | Your machine's LAN IP, e.g. `192.168.1.42:8080` |

**Critical:** `ADM_SFA_WEB_BIND` defaults to `127.0.0.1:8080` (localhost only) if not set — the other machine on the LAN **cannot reach it** unless you set this to the real LAN IP. This is an easy thing to miss.

Also edit `deploy/adm-sfa-backup.service` if you want the nightly backup (replace `<DATA_DIR>`, `<BACKUP_STAGING_DIR>`, and `<user@remote-host:/path>`). Can defer this to after the service is confirmed working.

---

## Step 6 — Verify the service file syntax

```sh
# Fill placeholders in a temp copy and verify:
systemd-analyze verify /path/to/filled-in-adm-sfa-web.service
```

No output = OK. Any output = fix before installing.

---

## Step 7 — Install and start

```sh
sudo cp deploy/adm-sfa-web.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl start adm-sfa-web.service   # start manually first, don't enable yet
sudo systemctl status adm-sfa-web.service  # must show "active (running)"
```

Check the logs immediately:
```sh
journalctl -u adm-sfa-web -n 50
```

What to look for in the logs:
- `Listening on ...` — good, it started
- `failed to open database` — data dir path is wrong or permissions are wrong
- `ADM_SFA_WEB_PASSWORD not set` — password file not loaded
- Any panic — unexpected; check the exact message

---

## Step 8 — Smoke-test before enabling on boot

From another machine on the LAN, open `http://<LAN_IP>:8080` in a browser:
1. The login page appears (not a connection error)
2. Log in with the password — settings page shows existing categories (proves it's reading the right DB, not a fresh one)
3. Navigate to Purchases — existing records appear
4. Navigate to Settings — locale picker shows current language

**The single most important check: do your existing records appear?** If you see an empty app, stop immediately — the service is pointing at the wrong data dir. `systemctl stop adm-sfa-web` and recheck Step 1 and 5.

---

## Step 9 — Enable on boot only after smoke-test passes

```sh
sudo systemctl enable adm-sfa-web.service
```

---

## Step 10 — Firewall (don't skip)

Binding to the LAN IP is only one layer. An actual firewall rule limits which hosts on the LAN can reach it:

```sh
# Example with ufw — adapt if using nftables/iptables:
sudo ufw allow from 192.168.1.0/24 to any port 8080 proto tcp
```

Without this, anything that can route to the machine can reach the app.

---

## Ongoing: nightly backup

Once the service is confirmed working, set up the backup timer:

```sh
sudo cp deploy/adm-sfa-backup.service deploy/adm-sfa-backup.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now adm-sfa-backup.timer
```

Test it runs manually first:
```sh
sudo systemctl start adm-sfa-backup.service
journalctl -u adm-sfa-backup -n 20
```

The backup script uses SQLite's online `.backup` command — it's safe to run while both the desktop and web apps are open (that's what WAL mode is for).

---

## Known limitations / accepted tradeoffs

- **Sessions reset on web service restart** — anyone logged into the web app gets logged out when the service restarts. No data loss; just mildly annoying. By design (the cookie signing key is generated fresh on startup).
- **Desktop doesn't see locale changes in real time** — if the web user changes the language, the desktop session shows the old language until the desktop is restarted. Accepted.
- **No per-user accounts** — single shared password. Both LAN users share one session. Documented design decision.
- **Documents created by web are owned by `adm-sfa`** — the desktop app can read them (because of group permissions), but can't delete them through the OS without being in the `adm-sfa` group. The app's own soft-delete path goes through the web service anyway, so this is fine in practice.
