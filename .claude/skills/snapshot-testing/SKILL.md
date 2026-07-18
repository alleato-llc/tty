---
name: snapshot-testing
description: Use when writing, updating, or fixing a snapshot or behavior test in tty — rendering chrome to a PNG and pixel-comparing, re-baselining a changed image, or diagnosing a flaky one. Covers the iced_test Simulator, the nextest serial-ui / default-filter setup, the -wgpu backend suffix, the re-baseline flow, and the fixed-clock gotcha. Apply whenever a `snapshot::*` or `behavior::*` test is involved.
---

# Snapshot + behavior tests (tty)

tty has two UI-level test kinds beyond plain unit tests:
- **`snapshot::*`** (`snapshot.rs`) — render the real chrome to a PNG and pixel-compare against a
  committed baseline. Catches visual regressions.
- **`behavior::*`** (`behavior.rs`) — drive `state`/`update()` with **pty-less** tabs (no shell),
  asserting on state transitions. No pixels.

Both build a `Tty` and exercise the real `view`/`update` paths.

## The nextest setup (`.config/nextest.toml`)

Snapshots render real pixels; their baselines are **backend-specific (wgpu) and machine-local**,
so they're **excluded from the default run**:
```toml
[profile.default]
default-filter = 'not test(snapshot)'
```
`snapshot::*` and `behavior::*` share a config dir + the GPU, so they run in a **`serial-ui`**
group (one at a time); everything else stays parallel. Commands:
```sh
cargo nextest run -p tty                              # unit + behavior (everyday)
cargo nextest run -p tty --ignore-default-filter      # whole suite incl. snapshots
cargo nextest run -p tty --ignore-default-filter -E 'test(my_surface_view)'   # one test
```

## Authoring a snapshot

Copy an existing test. Build the exact `Tty` state, render, compare:
```rust
#[test]
fn my_surface_view() {
    let mut tty = populated();                 // the shared fixture in snapshot.rs
    tty.show_x = true;                          // …set the exact state to capture…
    std::fs::create_dir_all("snapshots").expect("create snapshots dir");
    let mut sim = iced_test::Simulator::new(main_chrome(&tty));   // main_chrome = the app view
    let snap = sim.snapshot(&crate::state::theme(&tty)).expect("render snapshot");
    let matches = snap.matches_image("snapshots/tty-my-surface.png").expect("write/compare");
    assert!(matches, "snapshot `tty-my-surface` changed — delete its PNG to re-baseline");
}
```
`populated()` and `main_chrome()` are the shared helpers at the top of `snapshot.rs`. Add
`use`-free fields by mutating `tty` after `populated()`.

## Re-baselining (after an intentional visual change)

- The committed file has a **backend suffix**: `snapshots/tty-my-surface-**wgpu**.png` — *not* the
  bare name passed to `matches_image`. Find it: `ls tty/snapshots/ | grep <name>`.
- **Delete that PNG and re-run.** A missing baseline is **written and passes**. Re-run once more
  to confirm it now compares clean (not just first-write).
- Only delete the snapshots you meant to change; a diff in an unrelated one is a real regression
  (or the clock gotcha below).

## Gotchas

- ⚠️ **Fixed clock — the #1 flake.** A fixture seeded from `chrono::Utc::now()` or
  `SystemTime::now()` bakes wall-clock time into a pixel-exact image. It drifts, and **flips
  outright when a run crosses midnight** (this bit the `settings_history_*` snapshots). Seed a
  **fixed anchor** and pin the view's clock: `Tty::clock_override` (a `Option<u64>` epoch-ms),
  threaded into `now_ms()` / `age_from_epoch_ms`. Set both the fixture's seed and the override to
  the same fixed instant so absolute *and* relative time render deterministically.
- **New `Tty` field → update the `snapshot.rs` literal** (`populated()`) too, plus `state.rs` and
  `behavior.rs` — the 3-init trap (`error[E0063]: missing field`).
- **`Settings::save` is a no-op under `cfg(test)`**, so `behavior::*` can drive real `update()`
  without rewriting the developer's settings file. Rely on it; don't defeat it.
- Snapshots don't run in `cargo test` (the serial group can't be expressed there) — use nextest.

## rime's equivalent

rime has no pixel-diff gate; its visual check is `cargo run -p rime-demo` (a human looks) with an
optional `RIME_DEMO_SHOT=<path>` headless capture. See the `rime-widget` skill.
