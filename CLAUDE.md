# CLAUDE.md

Guidance for working in this repo. Read this before adding UI code.

## What this is

**tty** is a GPU-accelerated terminal built on Rust + [iced](https://iced.rs) 0.14.
It's the terminal counterpart of [`fed`](https://github.com/nycjv321/fed) and shares
its foundation (iced + the `rime` component kit). Three crates:

| Crate      | Role                                                                 |
|------------|----------------------------------------------------------------------|
| `cathode`  | Terminal **engine** — VT/ANSI `parser`, the `screen` grid model, a `pty` session, and a `wake` signal for output-driven repaint. Pure Rust, **no iced**. |
| `phosphor` | Terminal **widget** — a stateful iced widget rendering a `cathode` screen (color + attributes, scrollback, mouse select/copy), plus `input` (iced key → PTY bytes). Self-styles from a plain `TerminalStyle` (no theme-crate dependency). |
| `tty`      | The **app** — a minimalist tabbed terminal: thin glue over `cathode` + `phosphor` + `rime`. |

Names follow the retro CRT path: a **cathode** ray lights up the **phosphor** screen.

## The terminal is shared with fed-ide

`cathode` + `phosphor` are consumed by **fed-ide**'s integrated terminal panel via a
path dependency (`../tty/cathode`, `../tty/phosphor`), the same way `fed` consumes
`rime`. The dependency is **one-directional, fed → tty**: nothing here depends on
`fed` or its crates. So:

- **Keep `phosphor` theme-crate-free.** The host supplies a `TerminalStyle` (plain
  colors); fed-ide maps its `patina` theme into one, tty builds one from `rime` +
  `phosphor::ANSI_DEFAULT`. Don't reach for `patina` here — it lives in the fed repo.
- **Terminal behavior is shared.** A refinement to the engine or widget belongs in
  `cathode`/`phosphor`, so both the standalone app and the IDE panel get it.

## rime — the component kit (READ THIS)

`rime` is **our** reusable iced component kit, living *outside* this repo at
`../rime/rime` (a path dependency), shared with `fed`. It is the single source of
truth for the visual vocabulary: buttons, inputs, the `tabs` strip, `status_bar`,
etc.

> **Extend rime. Never fork, duplicate, or inline a UI component.**

rime components are **stateless** and self-style from a thread-local palette scope.
The host enters it once at the top of `view()`:

```rust
let _scope = theme::enter(self.theme.palette); // RAII; drops at end of view
```

Components call `theme::tokens()` at view-time. tty's `theme.rs` builds on rime's
shared catalog: `rime::theme::builtin_themes()` gives the 8 named chrome palettes
(Dracula, Nord, Gruvbox Dark, Solarized Dark/Light, GitHub Light, Neon Nights,
Phosphor) — the same list fed-ide shows — and tty pairs each with a `TerminalStyle`
(its base16 ANSI palette). A base16 import / panel edit becomes a "Custom" palette that
re-themes both the grid and the chrome. **Keep the catalog in rime**, not forked here,
so the two apps never drift. If you need a new reusable primitive, **add it to rime**,
then depend on it here — don't hand-roll a styled widget.

The **whole window fades when unfocused** ("Transparency On Blur"): `view()` runs the
palette and `TerminalStyle` through `fade_palette`/`fade_style` by
`state.window_opacity()`, and `state::theme()` fades the iced runtime theme to match —
iced 0.14 has no runtime window-opacity action, so the fade is uniform per-surface. The
opacity is clamped to `settings::MIN_OPACITY` (a 95% transparency cap) so the window
can never fade to invisible.

## Working in the app

tty is one iced 0.14 Elm loop: `Message` (`message.rs`) → `update()` (`update.rs`, mutates
state) → `view()` (`view.rs` + one `view/*.rs` per surface, `&Tty` → `Element`). The whole
app state is `struct Tty` in `state/types.rs`, with methods in `state.rs` (+
`state/{metrics,panes,scrollback,encrypted_history}.rs`); persisted config is `Settings`
(`settings.rs`). Tests: `behavior::*` drives `update()` with pty-less tabs; `snapshot::*`
renders chrome to a PNG.

Adding or changing a UI surface — a popover/drill-in, a status-bar cell, a settings
toggle, a widget — follows one repeatable procedure: **invoke the `ui` skill** for the
step-by-step (the wiring loop, the tty/rime boundary, popover/modal chrome, the settings
round-trip, snapshot re-baselining). Two traps worth knowing up front:

- **`Tty` is built by a full struct literal in three places** — `Tty::new` (`state.rs`),
  `behavior.rs`, and `populated()` (`snapshot.rs`). A new field must be added to all three
  or it won't compile (`error[E0063]: missing field`).
- **Snapshot fixtures must use a fixed clock.** A fixture seeded from `now()` bakes wall
  time into a pixel-exact image and flips across midnight; seed a fixed anchor and pin
  `Tty::clock_override` (threaded into `now_ms()` / `age_from_epoch_ms`).

Subsystems beyond this file: the status-bar metrics / drill-in popovers are surveyed in
`docs/ideas/status-bar-metrics.md`; the Env view + shell integration and the overall
render/state architecture are in `docs/ARCHITECTURE.md`.

## Tabs are pane trees

A tab is **not** one terminal — it's a `Tab { panes: pane_grid::State<Term>, focus }`
(`state.rs`), a tree of split panes that starts as a single pane. iced 0.14's built-in
`pane_grid` owns the split tree, drag-to-resize dividers, focus, and cardinal
`adjacent()` navigation, so we don't hand-roll any of it (and nothing about splits
belongs in rime — `pane_grid` is an iced widget, not reusable chrome). The "active tab,
focused pane" is the target of all input: `write_focused`/`resize_pane`/`paste`/the ⌘C
selection read `tabs[active].panes.get(focus)`; `drain_effects`/`reap_dead` walk **every
pane of every tab**. Split (`⌥⌘`+arrow) / focus-move (`⌃⌘`+arrow) / close (`⌘W`) are
keyboard chords handled directly in `update::handle_key`, like the other ⌘ shortcuts.
The same actions are reachable by **right-click**: a `mouse_area` per pane (and the tab
strip's right-press hook) opens a rime `context_menu` anchored at `state.pointer` (a
`PointerMoved` subscription tracks the cursor, since `mouse_area` reports a press but not
its position). `state.menu: Option<(MenuKind, Point)>` records which kind is open — a
**pane** menu (split + close pane) or a **tab** menu (new tab + rename + split + close
tab). Both target the active tab. The **tab strip is always shown** (even with one tab)
so there's always a tab to right-click. Because macOS delivers a **Ctrl+click** as
`Left+Control` (not `Button::Right`), `update` also opens the menu when `ActivateTab` /
`FocusPane` arrive with `modifiers.control()` held — so the menu is reachable without a
two-button mouse. **Rename** (`Tab.title` override + `state.renaming`) shows a focused
field under the strip; `Tab::label()` is the one place that resolves a tab's display name
(custom title → program/OSC title → shell name). `split_focused` spawns the shell; its spawn-free core
`split_with(dir, term)` is what the headless tests drive. The focus border only renders
when a tab has **more than one pane** — a lone pane shows none.

## Encrypted history (tty/src/history/ + cathode::history)

Opt-in persisted command history, encrypted at rest — design in
`docs/adr/0006-encrypted-history.md` (+ `0007` key sources/async startup,
`0008` untracked sessions); the key-derivation pipeline (both KDF forms, the
`HistoryKeys` hierarchy, where Skein sits, and the open refinement options)
is surveyed in `docs/history-keys.md` — read it before touching key
handling, and update it when the design moves. Invariants to preserve when
touching it:

- **Command text only, never output.** Output routinely contains secrets; nothing
  in `PersistedCommandEntry` may ever grow an output field.
- **Split stays split**: `cathode::history` is pure DTOs + an event queue on
  `TerminalScreen` — no crypto, no filesystem, no settings. All of that lives in
  `tty/src/history/{crypto,keychain,passphrase,segment,manifest,writer,reauth}.rs`.
- **Never start history on the UI thread.** The keychain read can block on an
  OS dialog and the KDFs are deliberately slow — every start goes through the
  async `begin_history_start`/`start_*_async` path (thread + oneshot, lazy).
  All fixed-at-enable choices (key source, KDF, cipher, the passphrase
  itself) live in the ONE enable dialog; the settings section keeps them
  greyed out until the feature is on. The dialog's keychain shape explains
  the OS prompt before it can appear. `Tty::new` must stay crypto-free.
- **Untracked means zero events at the source.** An untracked screen
  (`TerminalScreen::untracked`) queues no history events at all — suppression
  lives in cathode's single `queue_history_event` gate, never in a drain
  path. An untracked *session* is immutable until relaunch (enabling history
  mid-session persists the setting but starts nothing), does zero crypto
  (no key read, no seed), and must stay legible: ○ tab marker, title suffix,
  status chip, panel badges.
- **The passphrase KDF sidecar is load-bearing plaintext.** A malformed
  `tty.history.kdf.json` (or an unknown kdf tag) is an error, never a silent
  re-mint — a fresh salt locks the user out of an archive their passphrase
  still opens. The sidecar's recorded recipe is *authoritative* for an
  existing archive; the `history_kdf` setting only picks the recipe for new
  ones. Wrong passphrase surfaces as the existing `AuthFailed` (no verifier,
  no oracle); an empty archive accepts any passphrase by design.
- **Eviction is not deletion.** Only explicit user Clear/Delete emits events;
  `MAX_COMMAND_LOG` eviction and RIS reset must never tombstone the archive.
- **One writer.** The background writer thread is the sole writer of
  segment/manifest files; everything funnels through its channel. Don't add a
  second write path.
- **Fail closed, never silently-on.** Any start failure warns, disables for the
  session, and reverts the setting; the off-toggle never deletes; only the
  dialog-confirmed Reset action deletes.
- **`keyring` needs its platform features** (`apple-native`, `windows-native`,
  `sync-secret-service` — set in the workspace `Cargo.toml`). Without them it
  silently compiles a non-persisting mock backend and every run mints a new key.
- **Tests never touch the OS keychain or LocalAuthentication** — both side-effect
  the machine running them (a real keychain entry / a real auth dialog). The
  crypto/segment/manifest/writer/passphrase layers are tested against temp dirs
  (the passphrase source is the fully-testable one); keychain and the native
  prompt are manual-verification territory. `reauth::authenticate` and the
  `start_*_async` fns are deliberately lazy (nothing fires until the task is
  polled) — tests rely on that. `Settings::save` is a no-op under `cfg(test)`
  so behavior tests can drive real `update()` paths without rewriting the
  settings file of whoever ran them.
- Dev keychain gotcha: every ad-hoc `cargo build` is a new app to the keychain
  ACL, so reading a key stored by an older build can hit an allow/deny dialog (or
  block). `security delete-generic-password -s tty -a encrypted-history-key`
  resets the dev state; the signed `.app` has a stable identity.
- The second sibling path dependency: `../dorado/rust/crates/dorado-engine`
  (alongside `../rime/rime`) supplies the opt-in Threefish-256 cipher AND all
  key derivation (`dorado_engine::kdf`): `derive_from_password` + `validate`
  stretch the passphrase, and `derive_from_key_with` fans the master (keychain
  or passphrase) into the `HistoryKeys` hierarchy under a family-matched PRF
  (`settings::HistoryFanout`: BLAKE3 for ChaCha, Skein-512 for Threefish, user
  overridable, fixed at enable) — the master never encrypts anything directly;
  the manifest and segments each get their own child key.
  tty owns only the sidecar/salt scope and the domains, never re-implements
  the dispatch. dorado's self-contained password *container* format stays
  unused: its per-file salt would re-run the KDF on every file read. In UI
  text, "dorado" names the project — the cipher is displayed as
  "Threefish-256 (dorado)".

## iced

iced is pinned at **0.14**. Before writing iced code (the `phosphor` custom widget,
app wiring, subscriptions), read **`../rime/ICED.md`** — the patterns & gotchas plus
the 0.13→0.14 diff. The terminal repaints **on output**: the `cathode` read thread
calls `cathode::wake::signal()`, and each app runs one always-on subscription that
awaits it (no idle polling). Don't reintroduce a periodic tick.

## Build & test

[cargo-nextest](https://nexte.st) is the test runner; the snapshot tests are
categorized in `.config/nextest.toml` (a `serial-ui` group + a snapshot
`default-filter`, mirroring fed).

```sh
cargo build --bins
cargo nextest run                          # unit + behavior (the everyday command)
cargo nextest run --ignore-default-filter  # whole suite, incl. the snapshot
cargo clippy --all-targets -- -D warnings
```

`behavior::*` tests drive `tty::state`/`update` with pty-less tabs (no shell);
`snapshot::*` renders the chrome to a PNG (backend-specific baseline, excluded from
the default run). Re-baseline a snapshot by deleting its backend-suffixed PNG
(`snapshots/<name>-wgpu.png`, **not** the bare name in `matches_image`) and re-running —
a missing baseline is written and passes. Full story in `docs/ARCHITECTURE.md`; the `ui`
skill has the authoring + re-baseline procedure.

## App icon

tty ships a neon **"tty."** wordmark icon on the green **Phosphor** CRT palette (a
sibling to fed's and rift's neon marks). The SVG master + generated `AppIcon.icns` (10
sizes) live in `tty/assets/`; the app embeds `icon-512.png`.

`tty/src/app_icon.rs` (the same helper fed and rift use) sets the **macOS Dock** icon at
runtime via AppKit (`objc2`) from the first `root_view` render — post-launch, main
thread, `Once`-guarded, since a bare `cargo run` binary isn't an `.app` bundle and a call
in `main` gets reset by winit — and decodes the PNG into an `iced::window::Icon` for
Linux/Windows taskbars. The packaged `.app` gets `AppIcon.icns` from `ci/salpa-tty.yaml`'s
`icon:` field. To restyle: edit `assets/icon.svg`, re-render to 1024 PNG, then `sips` an
`.iconset` + `iconutil -c icns` (and refresh `icon-512.png`).

**No inline test modules.** A module `foo.rs` keeps its unit tests in a sibling
`foo_tests.rs`, wired up with `#[cfg(test)] #[path = "foo_tests.rs"] mod tests;` (the
test file opens `use super::*;`). Don't write `#[cfg(test)] mod tests { … }` inline —
same rule as fed and rime.
