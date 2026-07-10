§3.x — Item Negotiation Status
Purpose. Allow an item to be documented at the moment negotiation begins (common on Kleinanzeigen) without committing it to inventory or the EUR ledger until pickup is confirmed.
Data model.

Add status_id (FK) to the item table, referencing a small lookup table item_status.
item_status seed values: negotiating, bought. Closed, domain-fixed set; seeded at migration. (Lookup table rather than CHECK enum, for consistency with the first-class-entity convention, though the set is not expected to grow.)
New items default to negotiating.

Input process. Unchanged. All existing fields (category FK, purchase channel, seller info note, etc.) are captured identically. The only addition is a status selector defaulting to negotiating.
Lifecycle.

negotiating → bought: single field edit. This transition is the trigger for the EUR outbound/purchase ledger write. No ledger entry is created at item creation while status is negotiating.
negotiating → (dropped): standard soft-delete → documents/_deleted/. No ledger effect (none was ever written).
bought is terminal for this feature; reverting bought → negotiating is out of scope (and would require reversing the ledger entry — disallow for now).

Ledger constraint (non-negotiable). An item with status negotiating MUST NOT appear in any EUR ledger total or per-donor reporting breakdown. The outbound cash transaction is bound to the status transition, not to item creation.
View/reporting rules.

Inventory / distribution-available views exclude negotiating items.
Ledger totals and per-donor breakdowns exclude negotiating items.
A separate filter/view MAY list negotiating items for follow-up, but they are explicitly "not yet inventory."

Handoff flag for Claude Code. Verify the purchase/ledger write is gated on the status transition and not on item insert; confirm all inventory and reporting queries filter status_id = bought.

§3.y — Native Screenshot Capture & Filing
Purpose. From the item register, trigger the OS's native screenshot utility so the user selects a region (typically the Kleinanzeigen chat), and have the app receive the resulting image, auto-name it per the existing convention, and file it as a labeled document — eliminating the manual save/rename/duplicate step.
Scope. The app does NOT implement capture, region selection, or window targeting. It invokes the OS's native region-capture tool and consumes the resulting PNG. All capture UX is the OS's.
Trigger. A "Capture screenshot" button in the item register, associated with the current item record (existing or being created).
Capture mechanism (per OS).

The app invokes the platform's native region-select screenshot command, directing output to a temp path the app controls.
Linux (X11): e.g. import (ImageMagick) or maim -s; (Wayland): grim -g "$(slurp)" or spectacle -r.
Windows: snippingtool / PowerShell capture, or Snip & Sketch region mode.
macOS: screencapture -i -s <path>.
The specific command per platform is config-driven (a settings entry), so the tool can be swapped without code change. Default commands seeded per detected OS.

Result handling.

On successful capture, the app receives the PNG at the controlled temp path.
The file is renamed per the existing naming convention and moved into the documents directory.
A document record is created, associated with the item, and assigned a document label (config-driven — e.g. a negotiation_screenshot label; addable without schema change).
If the user cancels the OS tool (no file produced / non-zero exit), the app records nothing and shows a neutral "capture cancelled" state. No partial document.

Constraints.

No window targeting; region select only. The app cannot and does not attempt to capture a specific application's window.
Respects soft-delete: a filed screenshot is deletable via the standard soft-delete path only.
Capture command failure (tool missing, non-zero exit) surfaces a clear error and files nothing.

Settings additions.

Per-OS capture command string(s) in the settings view.
Optional: default document label to apply to captures.

Handoff flag for Claude Code. Confirm the settings view exposes the capture-command config; verify temp-path handling and the cancel/failure paths produce no orphan document records; confirm the document label used is drawn from document_label (config-driven), not hardcoded.

Both are additive and stay inside your existing conventions (lookup table, soft-delete, config-driven labels, settings-driven commands). Want these as an actual patch to SPEC.md — proper section numbers slotted in, plus the matching schema.sql deltas (item_status table, item.status_id FK, seed rows) — or keep them as standalone review blocks for your external audit first?
