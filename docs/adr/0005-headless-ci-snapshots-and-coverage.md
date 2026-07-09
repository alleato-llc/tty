# 0005 — Headless CI: tiny-skia snapshot backend + coverage gate

Status: accepted

## Context

CI already ran the unit + behavior tiers headlessly (`xvfb-run -a cargo nextest
run`) on every push, but the snapshot tier — `iced_test::Simulator` rendering real
pixels and byte-comparing them against a committed PNG — was excluded from CI
entirely (`.config/nextest.toml`'s `default-filter`). The reason was sound, not an
oversight: `Simulator` renders through `wgpu` by default, and `wgpu`'s output is
GPU/driver-specific — a baseline recorded on a developer's macOS/Metal machine
won't byte-match a Linux CI runner's software-GL path, so a naive "just run
snapshots in CI too" would either fail every run (comparing against the wrong
baseline) or require CI-specific baselines that still shift with the runner's GPU
driver version. There was also no coverage measurement at all — no visibility into
which parts of `cathode`/`phosphor`/`tty` had any test exercising them.

## Decision

**Force `iced_test`'s `tiny-skia` backend for CI, keep `wgpu` for local dev.**
`ICED_TEST_BACKEND=tiny-skia` (an env var `Simulator` already reads) makes it skip
`wgpu` and use `iced_tiny_skia` instead — a pure CPU rasterizer, already compiled in
today (`tiny-skia` is an `iced` *default* feature, so no `Cargo.toml` change was
needed). It has **no GPU or display dependency at all**: verified by running the
snapshot tier in a plain Linux container (`rust:bookworm`, no `xvfb`, no display)
and getting byte-identical PNGs across two independent fresh containers matching
the `ubuntu-latest` CI runner. This makes the pixels portable and deterministic —
not a workaround, a genuinely different (and better-suited-to-CI) rendering path.

- **New baselines, new suffix.** `iced_test` names the baseline file after
  `renderer.name()`, so forcing `tiny-skia` produces `snapshots/*-tiny-skia.png`
  automatically — no naming scheme invented here, no collision with the existing
  `*-wgpu.png` baselines (which stay, for local `cargo nextest run
  --ignore-default-filter -E 'test(snapshot)'` iteration on a dev machine with a
  real display).
- **Generated where they'll run**, not on a dev laptop. The `tiny-skia` baselines
  were generated inside the same kind of container CI uses, not exported from
  macOS — sidesteps any residual font-shaping/rasterization variance between
  platforms rather than assuming it doesn't exist.
- **A separate CI step, not folded into the existing `xvfb`-wrapped one.** The
  unit/behavior step still runs unforced (tries `wgpu` first, gets a display from
  `xvfb`) — nothing about its passing/failing depends on pixels, so there was no
  reason to touch it. The snapshot step is new, forces `tiny-skia`, and needs no
  `xvfb`.
- **Coverage (`cargo-llvm-cov`) as its own CI job**, `--workspace`-scoped (so it
  reports on `cathode`/`phosphor`/`tty` only, not `rime` or upstream deps), also
  forcing `tiny-skia` so the instrumented run exercises the snapshot tier too.
  `--no-report` on the test run, then two separate `report` calls (`--lcov
  --fail-under-lines 60` for the gate, `--html` for a human-browsable artifact)
  reuse the same collected profile data instead of running the suite twice.
- **60% lines as the gate**, not a stricter number. Measured coverage at the time
  this landed was ~63% lines; the low-coverage files are mostly *inherently* thin
  (`cathode::pty`/`wake` — real PTY/OS signal plumbing; `tty::main`/`subscription`/
  `app_icon` — an entry point, iced subscriptions, and platform AppKit calls) rather
  than under-tested business logic, so a much higher blanket threshold would
  either block unrelated PRs on unfixable numbers or force testing code that isn't
  worth testing (mocking a PTY, or an `objc2` Dock-icon call, buys little). 60%
  gates real regressions with headroom for that structural floor.

## Consequences

- Two rendering backends now exist side by side: `wgpu` (macOS dev, real display,
  what a developer actually looks at when iterating) and `tiny-skia` (CI, headless,
  portable). Both are legitimate, permanent — not "`tiny-skia` until CI gets a
  GPU." A future contributor should not try to unify them onto one backend.
- Any snapshot-affecting UI change now needs **both** baselines refreshed (delete
  the stale PNG, rerun once locally for `-wgpu`, once in a matching container for
  `-tiny-skia`) — a small ongoing cost for CI actually catching visual
  regressions, which it could not do before this ADR.
- The coverage gate can go up over time as genuinely-testable gaps close (e.g.
  `phosphor::input`'s ~30%, a pure key→bytes function that's more testable than
  its number suggests) — it should track "what's realistic," not sit fixed at 60%
  forever.
