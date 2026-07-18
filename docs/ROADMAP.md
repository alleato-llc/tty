# tty — roadmap & enhancement backlog

This is the running list of where tty is and where it could go. It exists to be
**reviewed and pruned** — not everything here should be built. Every item is tagged
with the pillar(s) it serves so we can keep the product **focused**:

- **§ simplicity** — works with zero config; minimal chrome; no feature creep.
- **⚡ speed** — native, GPU, output-driven; provably fast on real workloads.
- **✦ customization** — intuitive, visual, *fun* to make yours (not a config DSL).

Anything that doesn't clearly serve one of these is a candidate to **cut**, not build.

---

## Shipped

- Tabbed terminal (`⌘T`/`⌘N`/`⌘W`, `⌘1`–`⌘9`), close-last-quits, `exit`-aware.
- **Split panes** — `⌥⌘`+arrows split the focused pane in any direction, `⌃⌘`+arrows move
  focus, drag a divider to resize, `⌘W` closes the focused pane (→ tab → quit). Or
  right-click a pane (split / close pane) or a tab (new tab / split / close tab). Each
  pane is an independent shell — or a graduated metric view (see Machine stats); built
  on iced's `pane_grid`. (Option B below — splits only, no persistence.)
- **Tabs inside panes** — a split pane can itself hold multiple terminal tabs, so tabs
  work at both the window top level and within any split. `⌥⌘T` opens one in the focused
  pane, `⌥⌘]`/`⌥⌘[` cycle them, `⌥⌘W` closes one (a compact strip appears above the
  terminal only when a pane has more than one). Drag a pane-tab to reorder it, or drop it
  onto any part of another pane to **move it into that split** (every pane is a drop zone
  while a drag is in flight). Right-click one for new / rename / **detach** / close;
  detaching opens it in its own window and reattaching returns it to its origin group.
- Full ANSI rendering: 16 / 256 / truecolor + bold, dim, italic, underline, inverse.
- Scrollback + mouse select / `⌘C` copy.
- **Configurable scrollback** — a global max-lines setting (applies live to
  already-open tabs, not just new ones) and `⌘K` to clear it.
- **Scrollback search (`⌘F`)** — full-transcript find (not just the visible window),
  with a live "N of M" count and `Enter`/`⇧Enter` to step between matches, auto-
  scrolling the current one into view.
- **Scrollback History panel (`⌘⇧H`)** — a separate command/output browser: each
  shell command and its captured output are recorded live (no shell-integration
  escape codes needed) as an accordion `rime::table`, with its own text filter.
  Output capture is capped per command (a global default, overridable per command via
  a glob pattern, e.g. `"tail *" → 200`) so a streaming command (`tail -f`) just stops
  growing instead of recording forever, and full-screen apps (htop, vim) are excluded
  entirely via the alt-screen check.
- **Encrypted command history (opt-in, off by default)** — persist the Scrollback
  History panel's commands across launches: command text only (never output),
  encrypted at rest (ChaCha20-Poly1305, or dorado's Threefish-256 as an explicit
  opt-in) with a random key from the OS keychain or a passphrase-derived one
  (Argon2id, scrypt, or PBKDF2 — user's choice) where no keychain exists, one
  file per local day plus an
  encrypted index, written by a single background thread. Startup is async and
  narrated (no cold OS prompts, no UI freeze). The panel pages back
  through prior days; on macOS, opening it is gated behind Touch ID / the device
  password (once per launch + an optional idle interval). Disabling never deletes;
  a separate confirmed **Reset** action does. (ADRs 0006/0007.)
- **Untracked sessions** — incognito for command history: ⌘⇧T opens an untracked
  tab (live in the panel, badged, never persisted); a launch can start untracked
  via a setting, an each-launch chooser, or `tty --untracked` — zero crypto
  activity, immutable until relaunch, marked in the strip/title/status bar.
  (ADR 0008.)
- **Clickable links (URL autodetection)** — `⌘`-hover underlines a detected URL,
  `⌘`-click opens it directly, and a plain right-click offers **Open Link** / **Copy
  Link**.
- **Clickable `path:line[:col]`** — the same autodetection over compiler / linter /
  grep output: `⌘`-hover underlines a file reference, `⌘`-click resolves it against
  the shell's cwd (OSC 7) and opens it via a configurable editor command (Appearance
  → Terminal), defaulting to the system opener.
- **Command-finished notifications** — OSC 133 shell integration (`133;C`/`133;D`)
  drives a system notification (✓/✗ + command + duration) when a command finishes
  while the window is unfocused and ran past a threshold. Manual snippet by default;
  opt-in zsh auto-install (generated `ZDOTDIR`). All the shell-integration controls
  live under one master toggle in the **Shell** settings section.
- **OSC 133 semantic prompts** — on a cathode command-regions layer (mark positions
  pinned to stable line ids, surviving scrollback): **prompt-to-prompt navigation**
  (`⌘↑`/`⌘↓`, `⌘⇧↑`/`⌘⇧↓` for failures only), **failed-command flagging** (a red prompt
  marker on non-zero exit), **copy last command output** (`⌘⇧O` / pane menu), and an
  opt-in **prompt gutter** (a dot per prompt, red on failure). The command-finished
  notification above rides the same marks.
- **Env view (`⌘⇧E`)** — a movable, resizable popover listing the focused pane's
  environment variables. It opens **compact** (a masked key/value list + an Add button,
  scrolling when long) and expands (`+`/`−`, the metric drill-ins' glyph controls) to the
  full experience: filter, reveal-values toggle, and a source note. Click a row to copy
  `NAME=value`. With **zero setup** it shows the pane process's launch-time environment,
  read from the OS (`KERN_PROCARGS2` / `/proc/<pid>/environ`); the shell-integration hook,
  when enabled, upgrades it to a **live** view that re-captures `env` each prompt (gated by
  an `.on` flag) with no polling, and the popover labels which source you're seeing. The
  live capture is **opt-in** (`shell_integration.env_view`, off by default) so an unused
  install does no env work; the OS read is tty-side and touches nothing in the shell.
  **Editable** two ways: an **Add variable** modal that sets/unsets in this pane (a visible
  inject at the prompt, a further opt-in) and a persistent new-shells `[env]` overlay in
  Shell settings.
- **OSC 52 clipboard** — an app inside `tmux`/`ssh`/`vim` can write the system
  clipboard (`take_clipboard`, surfaced to the host each drain).
- Font zoom (`⌘±`/`⌘0`) with real PTY resize (SIGWINCH).
- **Output-driven repaint** — redraw on shell output, idle-silent (no polling tick).
- **Theme catalog** — 8 named themes (Dracula, Nord, Gruvbox Dark, Solarized
  Dark/Light, GitHub Light, Neon Nights, Phosphor) shared via rime so tty and fed-ide
  offer one identical list; persisted in `tty.toml`.
- **Font family + size** — a curated monospace dropdown and `⌘±`/`⌘0` (or the panel) size.
- **Transparency On Blur** — optional, configurable fade of the whole window (chrome +
  grid + text) when it loses focus, up to 95%; opaque while focused.
- **Hand-editable config (`tty.toml`)** — the GUI is primary, but the file is yours to
  edit: a round-trip save (via `toml_edit`) preserves your comments and layout, unset
  options are omitted (no null noise), a legacy `tty.settings.json` migrates on first
  run, a malformed file is backed up rather than reset, and an external hand-edit is
  **live-reloaded** when a tty window regains focus.
- Embeddable: `cathode` (engine) + `phosphor` (widget) also power fed-ide's panel.

---

## Foundations & customization (shipped)

### Tier 0 — table stakes (be a real daily driver)

Correctness on `vim`, `tmux`, `ssh`, `htop`, `fzf`, `less`, `git` matters more than any
feature count. All land in `cathode`/`phosphor`, so **fed-ide inherits every one**. This
tier is complete.

- [x] **Paste (`⌘V`) + bracketed paste (mode 2004)** — ⚡§ — read clipboard → PTY; wrap
      in `ESC[200~…ESC[201~` when the app enabled bracketed paste, so multi-line paste
      can't auto-execute.
- [x] **Alternate screen buffer (1049/47/1047, 1048 save-cursor)** — § — full-screen
      apps stop corrupting scrollback.
- [x] **Mouse reporting (1000/1002/1003 + 1006 SGR)** — forward click/drag/scroll to
      TUI apps; hold `⌥` to force-select instead.
- [x] **Cursor shape + blink (DECSCUSR `CSI Ps SP q`) + visibility (25)** — bar /
      underline / block as the shell/vim request.
- [x] **Window/tab title (OSC 0/2) + working dir (OSC 7)** — tab labels reflect the
      running program; OSC 7 enables **new-tab-in-same-directory**. — §
- [x] **Unicode width** — wide (CJK/emoji) cells occupy two columns with a spacer, so
      alignment stops drifting. (Full grapheme clustering: see backlog.)
- [x] **Bell / activity** — subtle visual bell + a per-tab activity dot.
- [x] **Engine robustness** (rides along): tab stops, `S`/`T` scroll, `L`/`M` ins/del
      line, `@`/`P`/`X` ins/del/erase char, `G`/`d` absolute moves, DECSC/DECRC,
      DECCKM (app cursor keys) — what `vim`/`less`/`htop` actually emit.

### Customization milestone

- [x] **Visual settings panel** — ✦§ — reuse rime's `settings` shell + `color_field`
      (the same kit fed's settings use): theme, font family/size — all GUI, no file to
      hand-edit.
- [x] **Theme catalog (8 themes, shared via rime)** — ✦ — the named-palette set lives in
      rime, so tty and fed-ide present one identical list; a custom/base16 palette reads
      as "Custom".
- [x] **Font family picker** — ✦ — a curated monospace dropdown (no free-form font DSL).
- [x] **base16 import** — ✦ — paste/import any [base16](https://github.com/chriskempson/base16)
      scheme and the 16 ANSI colors + fg/bg/cursor + chrome palette retheme instantly.
      Taps a huge existing ecosystem with zero learning curve.
- [x] **Transparency On Blur** — ✦ — a tooltipped slider fades the whole window (chrome +
      grid + text) up to 95% when it loses focus; opaque while focused. iced 0.14 has no
      runtime window-opacity action, so it's a uniform per-surface fade at view-time.
- ~~**Signature "phosphor" look** (retro CRT scanlines + screen warp)~~ — **dropped.**
      Built (an overlay + a real geometric warp) and reviewed; cut as not the look we
      want. The cathode→phosphor *naming* stays; there's no CRT effect in the app.

---

## Backlog — proposed, not committed (review & prune)

### High-leverage next
- **OSC 133** — ✦ — **fully shipped** (see the Shipped section): notifications,
  prompt-jump, failed-command flagging, output copy, a **jump-to-next-*failed*-command**
  (`⌘⇧↑`/`⌘⇧↓`), and an opt-in **persistent prompt gutter** (a dot per prompt, red on
  failure).
- **Env editing** — **shipped** (both halves): **set/unset in the active pane** from the
  Env view's **Add-variable modal** (a visible `export`/`unset` injected at the prompt,
  single-quote-escaped, gated on being at a prompt via OSC 133, zsh/bash), and a
  **new-shells overlay** (an `[env]` map in `tty.toml`, applied at spawn, no running shell
  touched). A *silent* live-apply (a `precmd`-drained control file) stays deliberately
  deferred — the visible inject is self-documenting and enough for the common case.
  Follow-ups if wanted: fish/csh inject syntax, and per-row edit/unset buttons in the list.
- **Clickable links (OSC 8)** — the current URL autodetection is text-pattern based
  (shipped, see above); OSC 8 would let an app mark an arbitrary label as a link
  (e.g. `ls --hyperlink`) instead of relying on the text looking like a URL.
- **Config file round-trip** — ✦§ — **shipped.** The GUI reads/writes a human-readable
  `tty.toml` (`toml_edit` + serde); a save merges into the on-disk document so hand-added
  comments and layout survive; a legacy `tty.settings.json` is migrated on first run; a
  malformed file is backed up rather than reset; and an external hand-edit is **live-
  reloaded** when a window regains focus. (GUI stays the primary path.)

### Rendering / fonts
- **Full grapheme clustering** — combining marks, ZWJ emoji sequences, skin-tone
  modifiers (per-cell text, not a single `char`).
- **Ligatures + font features** — ✦ — opt-in programming ligatures (Fira Code etc.).
- **Glyph atlas + damage rendering** — ⚡ — cache shaped glyphs; redraw only dirty rows
  (`screen` already tracks them). Defends "fast" on heavy output.
- **Throughput benchmark in-repo** — ⚡ — `cat` a 50 MB file / `yes | head`; make speed a
  number we defend. Optional perf HUD (fed's `FED_MEM` precedent).
- **Sixel / kitty graphics protocol** — inline images. Big surface; likely a "no" unless
  demand is real.
- **Reflow on resize** — rewrap scrollback to the new width (hard; nice-to-have).

### Workflow / app
- **Machine stats in the status bar** — ✦ — configurable CPU / memory / network /
  disk as compact canvas sparklines, reusing `fdtop`'s `prexp-core` sampler and
  `rime`'s chart kernel; an ambient peek line for the auto-hidden bar, and a
  click-through mini-fdtop system overlay as the north star. Phases 1-3 + the
  drill-in popover shipped: CPU + memory sparkline/number cells, network/disk
  throughput rate cells (macOS samplers via `prexp-core`/`prexp-ffi`), the ordered
  `status_bar_metrics` config list (add/reorder/style + sample interval),
  width-shedding, and a drill-in popover (click a sparkline for its full-size line
  chart over the retained history) with per-point hover, expand, border-resize, and
  drag-to-move. Since extended: three separately configurable CPU drill-ins (total
  / per-core grid / both), a per-core P/E cluster grid (each core's `cluster-type`
  read once via `prexp-core`'s `cpu_perf_levels()`), an optional pinned mode that
  keeps several popovers open at once, a fixed 0..100% gauge for the bounded
  metrics, text **Uptime** / **Session** cells (boot time via
  `system_boot_time_secs()`), a **Clock** cell, a **Load** cell
  (`system_load_average()`), a **Battery** gauge (`system_battery()`), a **Processes**
  cell (busiest process; drill-in a scrollable, sortable table via the light
  `process_summaries()`), **swap** in
  the memory drill-in (`MemoryInfo.swap_*`), configurable per-cell **warn/alarm
  thresholds** (past which the whole cell recolors), **scroll-to-page** through
  shed cells, and a press-hold **drag-to-reorder** edit mode with an insertion
  bar. The Processes cell since grew a per-process **detail drill-in** (open file
  descriptors + a live CPU chart via `snapshot_pid`, kept only while viewed) with
  right-click **copy path / PID / name** and **CPU-hog coloring** (the CPU% cell
  grades amber/red). And a drill-in can now **graduate into a pane**: the ⊞
  control splits it off — or *replaces* an existing pane (ending that shell after
  a prompt) — into a maximizable metric pane in the `pane_grid` (panes carry a
  `Pane` = `Term | Metric` content enum); it's gated by a setting and housed, with
  the rest of the metric config, in a dedicated **Metrics** settings section, plus
  a **highlight-focused-pane** toggle. Remaining: the Linux net/disk samplers, the
  peek line, the full mini-fdtop overlay. Design sketch:
  [`ideas/status-bar-metrics.md`](ideas/status-bar-metrics.md).
- **Profiles** — § — named tab presets (shell + command + cwd + theme). Keep it light.
- **Command palette (`⌘K`)** — § — new tab, theme, settings, search — one fuzzy entry.
- **Quick-open recent dirs / `ssh` hosts** — small launcher conveniences.
- **Find-as-you-type in scrollback with regex** — upgrade of Tier 0 search.
- **Configurable keybindings file** — ✦ — beyond the GUI editor.
- **Broadcast input** (type to all tabs) — only if multiplexing lands (see below).

### Platform / distribution
- Linux + Windows release parity (currently macOS-signed; archives exist).
- `.deb`/`.rpm`/`winget`/Homebrew cask.
- Crash/again-friendly session restore (reopen tabs + cwds).

---

## Multiplexing — where we landed

> Multiplexing = **splits/panes**, **ephemeral detach** (tear a tab into its own
> window), and/or **persistent sessions** (survive a crash/restart) *inside* tty, à la
> tmux/zellij/WezTerm. This was the single biggest fork in tty's identity, because it
> pulls against **§ simplicity**. Four coherent stances:

| Option | What it means | Status |
|---|---|---|
| **A. No multiplexing** | Lean on `tmux`/`zellij`. tty stays one clean window with tabs. | superseded by B |
| **B. Splits only** | In-window pane splits, no persistence. | **shipped** — a bounded `pane_grid` split tree, one `phosphor` per pane, no server/protocol. The `Term` model was extended into a per-tab pane tree exactly as scoped. |
| **C. Splits + ephemeral detach** | Tear a tab into its own OS window and dock it back — same process, no server. | **shipped** — a `daemon`-model multi-window tear-off; the owned `Tab` moves between windows. Detach via menu / tear-off drag; reattach via button / close / drag-dock (ADR 0003). A detached shell is ephemeral (dies with the app). |
| **D. Persistent sessions** | Detach/reattach that **survives a crash/restart** — a session server + serialization, a tmux replacement. | **out** — a different product. |

We shipped **B** and **C** the way the constraint demanded: no config language, no
session server. **C** is *ephemeral* multi-window only — the line ADR 0003 draws is that
a live tab can move between windows of the running process, but **D** (persistent
sessions surviving restart) stays off the table. The complementary bet still holds —
make tty *excellent under tmux* (correct mouse, OSC 52, titles, clipboard) so the combo
beats a heavyweight terminal.

---

## Non-goals (kept off the list on purpose)

A scripting/Lua config language · a plugin system · an embedded `ssh` client · GPU shader
zoo · tmux-style scripting/control mode. Each is how xterm/iTerm got heavy. The visual
settings panel + "great under tmux" is the focused answer instead.
