# 0008 — Untracked sessions (incognito for encrypted history)

Status: accepted

## Context

With encrypted history persisting commands across launches (ADR 0006), the
user needs the opposite gesture too: work that should *never* enter the
archive — per tab, and for a whole session — without toggling the feature
off and back on (and without trusting themselves to remember to). The model
is a browser's incognito window: visible while it's open, gone when it
closes, never written down.

## Decision

**One word everywhere: "Untracked."** Untracked commands still appear in the
live Scrollback History panel while their tab exists (useful within the
session, like history inside an incognito window) but are never written to
the archive and vanish with the tab. Because the user asked for the promise
to be *legible*, every surface says so: a `○` prefix in the tab strip, an
"— Untracked" window-title suffix (visible even with the strip hidden or
the tab detached), a status-bar chip (`untracked — not recording`), an
`· untracked` badge on the panel's rows with a caption explaining
session-only, and the settings History section states when the whole
session is untracked (and why, if `--untracked` forced it).

**Suppression lives in cathode, at the source.** `TerminalScreen::untracked`
(set at spawn, next to `set_pane_tag`) makes the screen queue *no history
events at all* — every push site routes through one gate
(`queue_history_event`). Producing nothing is fail-closed by construction:
no host drain path, present or future, can leak what was never queued. The
live `CommandEntry` carries an `untracked` flag for the panel badge, and
`Tab::untracked` mirrors the state for chrome rendering only. Splits inherit
the tab's flag (the promise is per-tab, not per-pane); detach keeps it (the
`Tab` moves wholesale).

**Per-tab: ⌘⇧T / "New Untracked Tab"** (tab context menu). Plain ⌘T stays
tracked; the shifted arm precedes it in `handle_key` so platforms that
deliver shifted chords as lowercase+SHIFT can't fall through.

**Per-session: a setting, a chooser, and a CLI flag.**
`Settings::history_session_start` = `"record"` (default) / `"ask"` /
`"untracked"`, resolved with the CLI by one pure function
(`startup_history_plan`, unit-tested as a matrix): `tty --untracked` beats
everything for one launch (logged, and attributed in the settings note);
otherwise the setting decides. "Ask" shows a chooser dialog at launch —
"Record this session's commands?" — whose backdrop-dismiss counts as *Stay
untracked* (fail-closed). An untracked session does the absolute minimum:
no writer, no seed, no key read, no keychain/passphrase prompt — zero
crypto activity, which also means an `--untracked` launch can never trigger
an OS keychain dialog.

**An untracked session is immutable until relaunch.** Toggling the history
setting on mid-session persists it (recording begins next launch) but does
not un-untrack the running session: the startup promise — "nothing typed
this session is saved" — would otherwise silently break for panes full of
already-typed commands. The settings section states this plainly. Commands
typed before the "ask" chooser is answered are likewise never persisted
(the writer only starts after "Record"), consistent with ADR 0007's
no-retro-recording rule.

## Consequences

- `cathode::screen::CommandEntry` gains `untracked` (live-only; the
  persisted DTO is untouched — nothing untracked ever reaches it).
- The reauth gate (ADR 0006) and Reset are orthogonal and unchanged; Reset's
  restart-fresh branch is skipped while the session is untracked.
- The tab-strip tooltip ("Untracked — commands in this tab are never saved")
  needs a small rime extension (a per-tab hint); deferred as a fast-follow —
  the marker, title, chip, and panel caption already carry the promise.
- The `tty-tab-menu` snapshot baselines were regenerated (the tab menu
  gained an item).
