# 0004 — Configurable scrollback + a Scrollback History panel

Status: accepted

## Context

Scrollback was a hardcoded `DEFAULT_SCROLLBACK: usize = 2000` in `cathode::screen` —
not a setting, not clearable, and there was no way to review it except scrolling the
live grid by hand. Two related asks followed: make the cap configurable (and add
`⌘K` to clear it), and add a proper full-transcript `⌘F` (a live match count,
`Enter`/`⇧Enter` navigation) instead of highlighting whatever happened to be
scrolled into view.

A third ask went further: browse **past commands and their output** separately, not
just a wall of raw text. That reframes the problem — a scrollback buffer is an
undifferentiated stream of terminal rows; "commands" is a concept the shell has
(via its line editor and prompt) that the terminal, as a dumb screen renderer,
does not.

**Rejected: OSC 133 semantic-prompt shell integration.** The standard way modern
terminals (kitty/iTerm/WezTerm/Ghostty) solve this is shell-injected escape
sequences marking prompt/command/output regions. It's the most capable answer —
and was rejected for this feature. It requires shell-specific integration scripts
(bash/zsh/fish each need their own snippet sourced into the user's rc file), doesn't
work retroactively for a shell that hasn't been configured, and is a much bigger
surface than "let me see my last few commands and what they printed." It stays on
the roadmap backlog for what it's *actually* good at (prompt-to-prompt jump,
flagging failed commands) — solving today's ask does not require it.

**Rejected: a monotonic position counter / sparse boundary markers.** An early
design tagged each scrollback row with a position counter and stored sparse
"command started at position N" markers, reconstructing command/output grouping
from the tags at read time. Simpler than OSC 133, but still more machinery than the
problem needs: a terminal only ever appends, so "which command is this output
under" is fully determined by *when* it was written relative to the last boundary —
no tagging or reconstruction required if the grouping is built **live**, as output
arrives.

## Decision

**Build `CommandEntry` structures live, with two hook points, and nothing else:**

- `TerminalScreen::mark_command_boundary(max_output_lines)` — the host calls this
  right before forwarding an `Enter` keystroke to the shell. Rather than reading
  the row immediately, it *queues* a boundary (`pending_boundaries`); the entry is
  created once `record_output_line` sees that row actually complete (the shell
  echoing the Enter) — capturing `current_row_text()` *then* (the fully-resolved
  command text — this is why it has to come from the *screen*, not the raw input
  stream: shell echo, history-recall, tab-completion, and backspace editing mean
  the keystrokes sent to the PTY don't equal the command text) into a new
  `CommandEntry { command, output: vec![], started_at, max_output_lines }` on a
  bounded `VecDeque<CommandEntry>` (`command_log`, capped at `MAX_COMMAND_LOG =
  500`). See "Follow-up: multi-line paste" below for why creation is deferred
  rather than immediate.
- `record_output_line`, called from inside `advance_row()` (every row-advance,
  wrap or newline) — resolves the front of `pending_boundaries` if this
  completing row is that boundary's own line, else appends the row to the open
  entry's `output` (unless it's already at that entry's `max_output_lines` cap, a
  no-op). Either way, the command's own line is never double-counted as its own
  first line of output.

This means **no explicit "streaming" concept**: a long-running command like
`tail -f` or `ping` simply stops growing its `output` past the cap — the same
mechanism that bounds any command's output also bounds a stream, with no special
case. Combined with the pre-existing alt-screen flag, `record_output_line` and the
live-grid append in `transcript_lines()` both skip entirely while `self.alt` is
true, so full-screen apps (htop, vim, less) capture **zero** output — fixing a real
bug where `transcript_lines()` had been including the live alt-screen grid (htop's
dashboard was leaking into "history" even though `scrollback` correctly excluded
it).

**Per-command output caps, global + glob override.** `cathode::commands` adds
`glob_match(pattern, text)` (`*`/`?`, fully anchored) and `resolve_output_cap
(command, overrides, default)`; `mark_command_boundary`'s caller resolves the cap
once, at command-start, from the app's settings (a global `default_output_lines`
plus an ordered list of `{pattern, max_lines}` overrides — e.g. `"tail *" → 200`).
Ported into `cathode` (a shared, public module) from what had been a private glob
matcher duplicated inside fed-ide, so tty and fed-ide resolve caps identically.

**A purpose-built `rime::table` widget, not `grid`.** `rime::widgets::grid` is
deliberately spreadsheet-shaped (per-cell selection, an inline cell editor,
resizable columns, row/column-letter chrome) — forcing it to render an accordion of
commands would mean fighting that shape, not reusing it. `table` (new in rime) is
the general "rows of records" counterpart: a header + virtualized, zebra-striped
body, whole-row selection/highlight, fixed-or-fill columns. The Scrollback History
panel computes a **flattened row list** per render — a command row, followed by one
row per output line when that command's index is in a `scrollback_expanded:
HashSet<usize>` — rather than adding variable-row-height/expand-collapse support to
`table` itself, keeping the widget itself simple and reusable for anything that's
just "rows of text" (logs, search results, file lists).

**Full-transcript find, decoupled from rendering.** `phosphor::find_matches(screen,
cols, query)` scans the *whole* transcript (scrollback + live grid) instead of only
the visible window, giving the host a match list to drive "N of M" and
`Enter`/`⇧Enter` navigation. `.scroll_to(Option<usize>)` is edge-triggered (applied
once per distinct target, tracked in widget state) so it brings a match into view
without fighting a manual scroll. `draw()`'s highlight is still viewport-scoped (only
visible rows are ever drawn) but now reads from the same `find_matches` list, not a
second scan.

## Consequences

- `command_log` and `scrollback`/`scrollback_times` are independent structures with
  independent caps (`MAX_COMMAND_LOG` vs. `max_scrollback`) — clearing one
  (`clear_scrollback`) clears both, but they don't share storage. This is a small
  amount of duplication (a command's rows exist in both `command_log` and, until
  evicted, `scrollback`) traded for keeping each structure simple and independently
  reasoned about.
- No shell-side integration of any kind — this works with an unmodified `bash`,
  `zsh`, `fish`, whatever the user runs, because the terminal derives everything
  from what it already renders. The cost: an in-progress edit to the command line
  (before Enter) isn't tracked as a "command" until the boundary fires, so there's
  no partial/live entry for a command still being typed — acceptable, since the
  panel is a history view, not a live composer.
- The design deliberately does not persist `command_log` to disk (matching the
  existing no-scrollback-persistence stance) — it's in-memory only, cleared with
  the rest of scrollback. *Superseded (2026-07-12): persistence now exists as an
  explicit, off-by-default opt-in — encrypted at rest, command text only, never
  output. See [ADR 0006](0006-encrypted-history.md).*

## Follow-up — multi-line paste (2026-07-09)

A pasted multi-line block (`⌘V`) is written to the PTY in one shot
(`Tty::paste`), entirely bypassing the keyboard path — so it could never call
`mark_command_boundary`. Without bracketed paste (mode 2004), the shell can't
tell a paste apart from typing: each embedded newline runs immediately, exactly
like a real Enter, just not through one. Since nothing marked a boundary for
those lines, all of it — every auto-run line's echo, execution, and output — was
misattributed as *output of whichever command was open before the paste* (or
silently dropped if none was). Expanding that one entry in the panel showed what
looked like several commands squashed into one's output.

**Decision: queue a boundary per complete pasted line, using its already-known
text — but only when the paste isn't bracketed.** `Tty::paste` now, before
writing anything: if `!bracketed_paste`, splits the text on `\n` (dropping a
final unterminated fragment — an in-progress line, same as normal typing isn't
"entered" yet) and calls a new `TerminalScreen::mark_command_boundary_with
(command, max_output_lines)` per complete line, resolving each line's own output
cap via `resolve_output_cap` exactly as a typed command would. A *bracketed*
paste is left untouched: the app declared it'll hold embedded newlines as
literal text in one edit buffer (not separate commands) until a real Enter, so
preemptively splitting it would invent spurious boundaries for what might be one
genuine multi-line command (e.g. a pasted heredoc). This can't perfectly cover
every shell's own paste-handling quirks (zsh's default `bracketed-paste-magic`
widget, for instance, auto-submits each complete line even *inside* a bracketed
paste) — that residual gap is accepted as a known limitation rather than chased
by guessing shell-internal config.

**This forced `pending_boundaries` to become a queue, not a single flag** — a
multi-line paste queues *several* boundaries at once, all before any of their
real echoes arrive (the whole paste is one synchronous write; the shell's
responses stream back asynchronously, one line at a time). Resolving them
naively FIFO — "the next row to complete always resolves the front boundary" —
breaks the moment a real *output* line completes before the next queued line's
echo does (e.g. pasting `"echo one\necho two\n"`: the row for `"one"`, cmd1's
output, completes *before* `"echo two"`'s own echo — a blind FIFO pop would
wrongly consume the `"echo two"` boundary right there). The fix: a known-text
boundary (`Some`, from a paste) only resolves when a completing row's text
actually *ends with* that known text (prompts prefix the row, so `ends_with`,
not `==`); a live-typed boundary (`None`, from a real Enter) still resolves on
the very next completion unconditionally, since nothing else can race between a
real Enter and its own echo.
