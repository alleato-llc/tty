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
- Full ANSI rendering: 16 / 256 / truecolor + bold, dim, italic, underline, inverse.
- Scrollback + mouse select / `⌘C` copy.
- Font zoom (`⌘±`/`⌘0`) with real PTY resize (SIGWINCH).
- **Output-driven repaint** — redraw on shell output, idle-silent (no polling tick).
- **Theme catalog** — 8 named themes (Dracula, Nord, Gruvbox Dark, Solarized
  Dark/Light, GitHub Light, Neon Nights, Phosphor) shared via rime so tty and fed-ide
  offer one identical list; persisted in `tty.settings.json`.
- **Font family + size** — a curated monospace dropdown and `⌘±`/`⌘0` (or the panel) size.
- **Unfocused-window transparency** — optional, configurable fade of the whole window
  (chrome + grid + text) when it loses focus; opaque while focused.
- Embeddable: `cathode` (engine) + `phosphor` (widget) also power fed-ide's panel.

---

## In progress (this milestone)

### Tier 0 — table stakes (be a real daily driver)

Correctness on `vim`, `tmux`, `ssh`, `htop`, `fzf`, `less`, `git` matters more than any
feature count. All land in `cathode`/`phosphor`, so **fed-ide inherits every one**.

- [ ] **Paste (`⌘V`) + bracketed paste (mode 2004)** — ⚡§ — read clipboard → PTY; wrap
      in `ESC[200~…ESC[201~` when the app enabled bracketed paste, so multi-line paste
      can't auto-execute.
- [ ] **Alternate screen buffer (1049/47/1047, 1048 save-cursor)** — § — full-screen
      apps stop corrupting scrollback.
- [ ] **Mouse reporting (1000/1002/1003 + 1006 SGR)** — forward click/drag/scroll to
      TUI apps; hold `⌥` to force-select instead.
- [ ] **Cursor shape + blink (DECSCUSR `CSI Ps SP q`) + visibility (25)** — bar /
      underline / block as the shell/vim request.
- [ ] **Window/tab title (OSC 0/2) + working dir (OSC 7)** — tab labels reflect the
      running program; OSC 7 enables **new-tab-in-same-directory**. — §
- [ ] **Unicode width** — wide (CJK/emoji) cells occupy two columns with a spacer, so
      alignment stops drifting. (Full grapheme clustering: see backlog.)
- [ ] **Scrollback search (`⌘F`)** — find within output, highlight + jump.
- [ ] **Bell / activity** — subtle visual bell + a per-tab activity dot.
- [ ] **Engine robustness** (rides along): tab stops, `S`/`T` scroll, `L`/`M` ins/del
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
- [x] **Unfocused-window transparency** — ✦ — an optional slider fades the whole window
      (chrome + grid + text) when it loses focus; opaque while focused. iced 0.14 has no
      runtime window-opacity action, so it's a uniform per-surface fade at view-time.
- ~~**Signature "phosphor" look** (retro CRT scanlines + screen warp)~~ — **dropped.**
      Built (an overlay + a real geometric warp) and reviewed; cut as not the look we
      want. The cathode→phosphor *naming* stays; there's no CRT effect in the app.

---

## Backlog — proposed, not committed (review & prune)

### High-leverage next
- **Shell integration (OSC 133 semantic prompts)** — ✦ — mark prompt / command / output
  regions: jump prompt-to-prompt, flag failed commands, "copy last command's output."
  The modern-terminal killer feature (kitty/iTerm/WezTerm/Ghostty). Pairs with OSC 7.
- **OSC 52 clipboard** — copy from inside `tmux`/`ssh`/`vim` to the system clipboard.
- **Clickable links (OSC 8 + URL autodetection)** — ✦ — `⌘`-click to open; underline on
  hover. (Storage lands in Tier 0; UI here.)
- **Config file round-trip** — ✦§ — the GUI writes a human-readable `tty.toml`; power
  users can hand-edit; live-reload on change. (GUI stays the primary path.)

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

## Under consideration: multiplexing (needs a decision)

> You asked to think about this more. Here's the framing. Multiplexing = **splits/panes**
> and/or **persistent sessions** (detach/reattach) *inside* tty, à la tmux/zellij/WezTerm.

It's the single biggest fork in tty's identity, because it pulls directly against
**§ simplicity**. Three coherent stances:

| Option | What it means | Pro | Con |
|---|---|---|---|
| **A. No multiplexing (current)** | Lean on `tmux`/`zellij`. tty stays one clean window with tabs. | Maximal focus; smallest surface; fastest to a great 1.0. | Users who want panes reach for tmux (which many already run). |
| **B. Splits only** | In-window pane splits (`⌘D`/`⌘⇧D`), no persistence. | The 80% people actually want; no server/protocol. | Real complexity: focus model, per-pane PTY/resize, layout tree, copy-mode-ish nav. Pulls chrome toward iTerm. |
| **C. Splits + persistent sessions** | Detach/reattach, survive crashes — a tmux replacement. | A genuine differentiator; "tmux without the config." | Large: a session server, serialization, reconnect protocol. Risks becoming the thing we said we wouldn't. |

**Leaning:** **A for 1.0**, and *consider* **B** later **only if** it can be done without
a config language or a server — a bounded split tree reusing `phosphor` per pane, with
the existing `Term` model extended. **C** is probably a "no" — it's a different product.
A cheap middle path: make tty *excellent under tmux* (correct mouse, OSC 52, titles,
clipboard) so the combo beats a heavyweight terminal. **Decision pending your call.**

---

## Non-goals (kept off the list on purpose)

A scripting/Lua config language · a plugin system · an embedded `ssh` client · GPU shader
zoo · tmux-style scripting/control mode. Each is how xterm/iTerm got heavy. The visual
settings panel + "great under tmux" is the focused answer instead.
