# tty

A fast, focused, GPU-accelerated **terminal** written in Rust — tabs, scrollback,
mouse select & copy, full ANSI color, and output-driven repaint (it redraws when
the shell writes and sleeps when it doesn't). The terminal counterpart of
[`fed`](https://github.com/nycjv321/fed), built on the same `iced` + `rime`
foundation.

> The name nods to the old `tty` — but it ships as **`Tty.app`** so the bare `tty`
> binary never shadows the POSIX `tty(1)` command.

## Crates

This is a Cargo workspace of three crates — engine, widget, app — named for the
retro CRT path *cathode ray → phosphor glow*:

| Crate      | Role                                                                 |
|------------|----------------------------------------------------------------------|
| `cathode`  | The terminal **engine**: a VT/ANSI parser, the screen-grid model, a PTY session, and a wake signal. Pure Rust, no iced — embeddable by any front-end. |
| `phosphor` | The terminal **widget**: a stateful `iced` widget that renders a `cathode` screen (color + attributes, scrollback, mouse select/copy), plus iced-key → PTY-byte translation. |
| `tty`      | The **app**: a minimalist tabbed terminal — thin glue over `cathode` + `phosphor` + `rime` chrome. |

`cathode` + `phosphor` are also consumed by **fed-ide**'s integrated terminal
panel (by path dependency, the way `fed` consumes `rime`), so refinements to the
terminal land in both the standalone app and the IDE at once. See
`docs/ARCHITECTURE.md` and `docs/adr/0001-crate-split.md`.

## rime (and dorado)

The chrome (tab strip, status bar) comes from **rime**, our shared `iced`
component kit — a sibling path dependency at `../rime/rime`
([`alleato-llc/rime`](https://github.com/alleato-llc/rime)). Lay it down next to
this repo so the path dependency resolves. **Extend rime; never fork a UI
component.**

A second sibling path dependency, `../dorado/rust/crates/dorado-engine`
([`alleato-llc/dorado`](https://github.com/alleato-llc/dorado)), provides the
optional Threefish-256 cipher for encrypted history (see below) — lay `dorado`
down next to this repo too.

## Build & run

[cargo-nextest](https://nexte.st) is the test runner (the snapshot tests are
categorized in `.config/nextest.toml`):

```sh
cargo run -p tty                           # launch the terminal
cargo build --bins
cargo nextest run                          # unit + behavior
cargo nextest run --ignore-default-filter  # whole suite, incl. the snapshot
cargo clippy --all-targets -- -D warnings
cargo llvm-cov nextest --workspace --ignore-default-filter  # coverage (cathode/phosphor/tty)
```

CI runs all three test tiers headlessly, including snapshots (forcing `iced_test`'s
`tiny-skia` software backend — no GPU/display needed, unlike the `wgpu` backend used
for local dev) — see `docs/adr/0005-headless-ci-snapshots-and-coverage.md`.

## Keys

- `⌘T` / `⌘N` new tab · `⌘⇧T` new **untracked** tab (its commands are never saved
  to encrypted history — marked ○ in the strip) · `⌘1`–`⌘9` jump to tab
- `⌥⌘`+arrows split the focused pane (←/→/↑/↓) · `⌃⌘`+arrows move focus between panes ·
  drag a divider to resize
- **right-click** (or **⌃-click**) a pane for a split menu, or a **tab** for
  new-tab / rename / **detach** / split / close-tab
- **reorder tabs**: drag a tab sideways across the strip and it moves live to the slot
  under the pointer
- **rename a tab**: right-click → **Rename tab…**, type a name (Enter commits, blank
  reverts to the shell/program title, Esc cancels)
- **detach a tab into its own window**: right-click → **Detach Tab**, or drag a tab down
  out of the strip. Dock it back with the **Reattach** button, by closing the window, or
  by dragging the window onto the tab strip. (Detached terminals are ephemeral — they
  close with the app.)
- `⌘W` close the focused pane (in the main window the last pane → close the tab → quit; in
  a detached window the last pane → close the window)
- `⌘+` / `⌘−` font zoom · `⌘0` reset
- `⌘C` copy the selection (drag to select); `Ctrl+C` stays a real SIGINT to the shell
- line editing: `⌥←/→` move by word · `⌘←/→` to line start/end · `⌥⌫` delete a word ·
  `⌘⌫` delete to line start (sent to the shell's line editor)
- `⌘F` find in scrollback — `Enter`/`⇧Enter` jump to the next/previous match (shown as
  "N of M") · `⌘↑`/`⌘↓` jump to the previous/next command prompt (`⌘⇧↑`/`⌘⇧↓` to jump
  between failed commands only) and `⌘⇧O` copy the last command's output (all need OSC 133
  shell integration) · `⌘K` clear the focused pane's scrollback · `⌘⇧H` open **Scrollback
  History** (an accordion table of past commands + their captured output, its own
  search, double-click a row to copy the command; with **encrypted history** enabled
  it also pages back into previous days' commands, gated behind Touch ID / your
  password on macOS) · `⌘,` settings · wheel to scroll back through history
- `⌘`-hover underlines a URL in the output; `⌘`-click opens it directly, or right-click
  it for an **Open Link** / **Copy Link** menu
- `⌘`-hover also underlines a `path:line[:col]` reference (compiler / linter / grep
  output); `⌘`-click opens it in your editor. The path is resolved against the
  shell's working directory (from OSC 7). By default the file goes to the system
  opener; set an **open-file command** in Appearance → Terminal (e.g.
  `code -g {file}:{line}:{col}` or `vim +{line} {file}`) to jump to the exact line
- **Command-finished notifications** — when a command finishes while the window is
  unfocused and it ran longer than a threshold, tty posts a system notification
  (✓/✗ from the exit code, the command, and how long it took). This needs OSC 133
  shell integration: paste the snippet from the **Shell** settings section into your
  `~/.zshrc`, or turn on **auto-install** there (zsh; off by default) to have tty
  wire it up for new shells
- **Env view (`⌘⇧E`)** — a movable, resizable popover (like the metric charts, so it
  stays open beside the terminal) listing the focused pane's environment variables;
  click one to copy `NAME=value`, toggle **Reveal values** to unmask. It reads what the
  shell captures each prompt (same shell integration as above), so it tracks the shell
  across commands — read-only for now

## Customize

`⌘,` opens a visual settings panel (the primary path — no config file needed). Its **Appearance**
section is split into sub-tabs — Theme, Tabs, Status bar, Terminal, Window — so each
shows a focused pane instead of one long scroll. Pick one of 8 named
themes (the same catalog fed-ide ships, shared via rime) or import any
[base16](https://github.com/chriskempson/base16) scheme, choose a monospace font and
size, toggle **Highlight active tab** (accent ink vs. a subtler emphasis) and
**Highlight the focused pane** (the accent border on the focused pane in a split
tab — turn it off for a flat, borderless split), and the
**auto-hiding status bar** (on by default — it floats back in when the pointer
nears the bottom edge so the grid gets the full height; turn it off to pin it, or
**Disable status bar** to drop it entirely). The **Window** tab keeps the window
**on top** of other windows and fades it with two transparency amounts —
**When Active** (up to 50%, so an in-use window stays readable) and **On Blur**
(up to 95%, when it loses focus). A **Keys** section lists every keyboard shortcut
for reference. A dedicated **Shell** section holds the whole **shell integration
(OSC 133)** group behind one master toggle — notifications, prompt-jump,
failed-command flagging, output copy, and an opt-in **prompt gutter** (a dot beside
every command prompt, red for a failure) — all off at once when it's disabled.
Settings persist to a hand-editable `tty.toml` in the config dir
(`~/.config/tty/` or the platform equivalent). You can edit it directly — the GUI
does a round-trip save, so your comments and layout survive, and an edit you make in
another editor is picked up live the moment you switch back to a tty window. A
malformed file is backed up rather than overwritten, and an existing
`tty.settings.json` from an older build is migrated automatically on first launch.

A dedicated **Metrics** section (above History) configures **machine stats in the
status bar** (off until you add one): CPU, memory, and network / disk throughput
as compact canvas
sparklines or plain numbers, in an order you set (add, remove, reorder, choose a
per-metric style, and a sample interval). CPU and memory grade their color by
load; the network and disk rates auto-scale to their own recent peak, and the two
directions can share a single sparkline (**Net I/O**, **Disk I/O**). **Uptime**
(since the machine booted) and **Session** (since this terminal launched) are text
cells that show a compact `up 3d 4h` and drill into the full breakdown, a
**Clock** cell shows the current time with configurable formatting (12/24-hour,
seconds, date), a **Load** cell sparklines the 1-minute load average and drills
into the 1/5/15-minute triple, and a **Battery** cell shows charge on a gauge
(colored by level) with the charging state and time estimate in its drill-in, and a
**Processes** cell shows the busiest process (`↑ Chrome 92%`) and drills into a
scrollable, click-to-sort table of every process (by CPU, absolute memory, or
name), busy processes grading their CPU% amber (≥60%) then red (≥85%) so hogs
stand out. Right-click a row for a menu: **View Process** (drills into
that one process — a live CPU chart whose history is kept only while you're
looking, its memory and thread count, and the scrollable list of its open file
descriptors, each right-click-to-**Copy path**), plus **Copy path** / **Copy
PID** / **Copy name** for the process itself. `‹ Back` or `Esc` returns to the
list.
When the window is too narrow to hold every cell the bar sheds them from the
right rather than wrapping; **scroll over the bar** to page a sliding window
through the shed cells (chevrons show when there's more off each edge). The graded
cells (CPU, memory, battery) recolor by load with **configurable caution/alarm
thresholds** per cell — past a threshold the whole cell, label included, recolors
so it catches the eye. A quick **click** on a cell opens its drill-in; **press and
hold** one (~3s, configurable 1.5–5s) to enter a live **drag-to-reorder** edit
mode — the widgets outline, the held one lifts, and an insertion bar shows where
it'll drop. Rearrange freely, then press `Esc` or click empty bar space to
finish. Click any
sparkline to **drill in** — a small popover shows that
metric's full-size line chart over its retained history, with the current
readout; hover a point to read its value, hit **+** for a large centered chart,
drag any **border** to resize, and click away or press `Esc` to close it. CPU is
offered as three separate cells — **CPU** (the aggregate chart), **CPU Cores**
(a per-core sparkline grid grouped into Performance and Efficiency cores), and
**CPU (all)** (both) — so you pick the view you want. Turn on **Keep metric
popovers open** to pin several side by side at once (each with its own **×**;
`Esc` closes all). Clicking a metric's cell while that metric is already shown (as
a popover or a pane) does nothing — no duplicate opens; a different metric's cell
still works. To keep one permanently in view, **graduate it into a pane**:
the popover's **⊞** control moves the drill-in out of the floating overlay and
into a real split pane (Left / Right / Up / Down) beside the terminal, resizable
by dragging the divider like any pane. **Replace a pane…** in the same menu
instead swaps an *existing* pane for the metric — the grid dims, you click the
pane to take over, and replacing a live terminal asks first (its shell ends and
its scrollback is lost). A metric pane has its own maximize (fill
the terminal area) and close controls, and `⌘W` closes it too. (The ⊞ control can
be turned off in the Metrics section for those who prefer popovers only.) The
samplers reuse the sibling `fdtop` project's `prexp-core`
(network / disk and the per-core P/E split are macOS-only for now). Design sketch
in `docs/ideas/status-bar-metrics.md`.

A **History** section holds the opt-in **encrypted command history** (off by
default): persist the Scrollback History panel's commands across launches —
command text only, never captured output — encrypted at rest. Flipping the
toggle on opens one **enable dialog** carrying every fixed-at-enable choice
(the section shows them greyed out until then): the **key source** (your OS
keychain, recommended — or a passphrase, for platforms without a usable
keychain, stretched with your choice of KDF: Argon2id, recommended, or
scrypt / PBKDF2-SHA256; you set the passphrase right in that dialog, and
each later launch starts *locked* until you enter it — tty says plainly that
nothing is recorded while locked) and the **cipher** (ChaCha20-Poly1305,
recommended, or the sibling dorado project's Threefish-256 construction — a
sound design, but not independently audited) and the **key fan-out PRF** that
splits the key into per-file subkeys (Auto matches the cipher's family —
BLAKE3 with ChaCha20-Poly1305, Skein-512 with Threefish — or override it;
both are equally strong). The dialog explains upcoming
OS prompts before they appear, and the key is read off the UI thread so the
window never freezes behind a hidden dialog.
Set how often macOS should re-ask for Touch ID / your password before opening
the panel (always at least once per launch), choose what **startup** does
(record right away / ask each launch / start untracked), browse the archive
right in the section (**View archived commands…** drills into a full-height,
scrollable browser behind the same Touch ID gate; right-click a row to
**Copy** the command — just the command, the captured shell prompt is
stripped — or **Delete…** it — deletion asks for confirmation first —
double-click to copy, "Load older day" to page back, "‹ Back" to return to
the settings), and — as a separate, confirmed action — **Reset encrypted
history** to permanently delete the whole archive. Turning the toggle off
only stops recording; it never deletes anything.

**Untracked sessions**: `⌘⇧T` opens an untracked tab — its commands stay in
the live panel while the tab exists (badged "untracked") but are never saved,
like an incognito window. A whole launch can start untracked via the startup
setting, the launch chooser, or `tty --untracked` (which also skips reading
the key entirely); an untracked session stays untracked until relaunch, even
if you flip history on mid-session. Design details in
`docs/adr/0006-encrypted-history.md`, `0007` (key sources, async startup),
and `0008` (untracked sessions).

## Landing page

`web/` is a static [Astro](https://astro.build/) site deployed to
`tty.alleato.dev` via `salpa` (see `salpa.yaml` and
`.github/workflows/deploy-site.yml`).
