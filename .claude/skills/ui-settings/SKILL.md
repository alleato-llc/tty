---
name: ui-settings
description: Use when adding or changing a tty setting — a toggle, a config field, or a settings-panel section — including the serde + toml_edit round-trip, the resolver + master-gate pattern, migrating renamed/moved keys, and wiring the control into the settings UI. Apply whenever you touch `Settings` in `settings.rs` or a section in `view/settings.rs`.
---

# A settings toggle / section

`Settings` (in `settings.rs`) is serde + `toml_edit` — a comment-preserving round-trip, so a UI
edit rewrites `tty.toml` without clobbering the user's comments/formatting.

**Shared loop** (full detail in the `ui` skill): `Message`→`update`→`view`→`state`. Note a
*persisted* field is loaded from disk so it does **not** go in the three `Tty` literals — but
any *UI draft* state you add (a text field being typed) is a `Tty` field and does (the 3-init trap).

## Field

Add it to the relevant struct. Optional fields skip serialization when unset:
```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub my_flag: Option<bool>,
```
A nested group uses its own predicate: `#[serde(default, skip_serializing_if = "MyGroup::is_empty")]`.

## Group + gate + resolver

Group related flags in a sub-struct (see `ShellIntegration`) and expose a **resolver** on
`Settings` that applies a master gate + per-field defaults:
```rust
pub fn my_group(&self) -> ResolvedMyGroup { … }   // callers read the resolved value, never the raw Option
```
Master-gating (feature off ⇒ every sub-flag resolves off) lives in the resolver, so UI and
runtime agree. Read the *resolved* value at view time — don't cache it — so the preview updates
the instant a toggle flips.

## Migration (renamed / moved keys)

Keep the old key readable but non-writing, and fold it forward:
```rust
#[serde(default, skip_serializing)]           // reads old files, never re-writes it
pub old_key: Option<bool>,
```
plus a `migrate_*` step mapping it to the new shape. A malformed value is an **error**, never a
silent re-mint (a re-minted default can lock a user out — see the history-KDF-sidecar rule in
`tty/CLAUDE.md`).

## UI (`view/settings.rs`)

The settings view dispatches on `state.settings_section` (`match state.settings_section { … }`).
Add controls to the right section fn (e.g. the Shell section) using rime `toggle` / `select` /
`stepper` / `text_field`, wired to `Set*` messages. Each calls a setter that persists:
```rust
Message::SetMyFlag(v) => state.set_my_flag(v),
// state.rs:
pub fn set_my_flag(&mut self, v: bool) { self.settings.my_flag = Some(v); self.settings.save(); }
```
Group a feature's controls under a `section(...)` / `caption(...)` header; grey out dependent
controls until the master toggle is on.

## Gotchas

- **`Settings::save` is a no-op under `cfg(test)`** — so `behavior::*` tests drive real
  `update()` paths without rewriting the developer's own settings file. Don't defeat it.
- Nested structs serialize **inline** in `toml_edit` (`shell_integration = { … }`, not a
  `[shell_integration]` block). Expected; match existing structure and fix test assertions to
  the inline form.

## Snapshots + verify

A new/reordered control changes the settings snapshots — re-baseline them. Authoring, the
`-wgpu` re-baseline flow, and verify commands are in the `ui` skill. Reference example: the
`[shell_integration]` group (`env_view` / `env_editing`, master-gated), its resolver, the flat-key
migration, and the Shell settings section.
