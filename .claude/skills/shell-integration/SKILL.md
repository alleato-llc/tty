---
name: shell-integration
description: Use when working on tty's shell integration — OSC 133 semantic prompts (prompt-jump, failed-command flagging, the command-regions layer in cathode), the zsh hooks / auto-install, per-session env capture, or injecting export/unset into a running shell. Apply whenever you touch `tty/src/shell_integration.rs`, the OSC 133 handling in `cathode/src/screen.rs`, or the env inject path in `state.rs`.
---

# Shell integration (OSC 133 + env)

Cooperative integration between tty and the shell. The whole feature is **opt-in**
(`shell_integration.enabled`, off by default) so an install that doesn't use it pays nothing.
Two layers, on purpose:

- **cathode** owns the OSC 133 *engine* (shared with fed-ide's terminal panel — keep it there,
  not in tty).
- **tty** owns the hooks, the env channel, injection, and the UI.

## OSC 133 in cathode (`cathode/src/screen.rs`)

The shell emits semantic-prompt marks; cathode parses them in `osc_dispatch` under a
`b"133" if self.honor_osc133 =>` guard (`set_honor_osc133` gates the whole thing):
- `133;A` / `133;B` — prompt start. `133;C` — command output start. `133;D[;code]` — finished
  (+ optional exit code).
- Positions are stored as **global line ids** (`lines_scrolled + cursor_row` at mark time) so
  they survive scrollback eviction; `command_regions()` converts them back to current-buffer
  rows. Types: `CommandMark` / `CommandRegion` / a `PendingMark` for the in-flight command.
- `command_running()` — true between `133;C` and `133;D`. **Load-bearing**: injection and
  anything that "types at the prompt" must gate on `!command_running()` so it never types into a
  foreground program.

This layer is pure cathode (no iced, no tty) — a refinement here reaches both the app and the IDE
panel.

## The hooks (`tty/src/shell_integration.rs`)

- `zsh_snippet(env_capture: bool)` — the pasteable `~/.zshrc` hooks. **Always** emits the OSC 133
  marks (precmd/preexec); adds the `_tty_capture_env` dump **only** when `env_capture`, so a shell
  does no env work unless the Env view is on. This is what the settings preview + Copy show.
- `autoinstall_env(shell, env_capture)` — best-effort auto-wire via a `ZDOTDIR` shim
  (`zsh_zdotdir`); returns env vars to hand the spawned shell.
- `env_channel_path()` — the per-session env file under a `0700` temp dir (`$TTY_ENV_FILE`).

Wiring at spawn (`state.rs::spawn_term`): `set_honor_osc133(integration.enabled)`; hand the shell
`TTY_ENV_FILE` **only** when `env_view` is on; apply `autoinstall_env` when auto-install is on.

## Env capture → the Env view

The hook writes `env` to `$TTY_ENV_FILE` each prompt, but only while a `<file>.on` flag exists
(so capture is event-driven off command boundaries, no polling, and off unless the view asked).
tty reads it on redraw. **The Env view UI itself is the `ui-popover` skill** (it also has an
OS-read fallback for when the hook isn't installed). This skill covers the shell side.

## Injection (`state.rs::inject_env*`, `tty/src/env.rs`)

"Set/unset in this pane" types a **visible** `export`/`unset` at the prompt (self-documenting):
- `env::export_command(name, value)` single-quote-wraps the value with `'\''` escaping; `unset_command`
  for unset. `is_valid_name` rejects a name that could smuggle shell syntax (so the value stays inert data).
- `inject_env` gates on **both** `shell_integration().env_editing` (opt-in, off by default — it
  types into your shell) **and** `!command_running()` (never into a foreground program), then
  `write_focused`.
- The cooperative *silent* channel (a `precmd`-drained control file) was deliberately **deferred** —
  the visible inject is enough and self-documenting. Don't add a second, hidden inject path
  without revisiting that decision.

## Settings

The `[shell_integration]` group (`enabled` master + `autoinstall` / `notify` / `notify_min_seconds`
/ `gutter` / `env_view` / `env_editing`) is master-gated by a resolver; the preview snippet is
conditional on the resolved flags. Wiring a toggle is the **`ui-settings`** skill; keep the
snippet/preview in sync with what the resolved flags actually do.

## Tests

`shell_integration.rs` snippet output is unit-tested (sibling `*_tests.rs`); OSC 133 parsing is
tested in `cathode/src/history_tests.rs` / screen tests. A UI change (gutter, notification, the
Env cell) is a snapshot — see the `snapshot-testing` skill.
