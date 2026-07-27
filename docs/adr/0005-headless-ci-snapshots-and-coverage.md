# 0005 — Headless CI: tiny-skia snapshot backend + coverage gate

Status: accepted — **amended 2026-07-26** (see the amendment at the end: the
baselines' portability claim in the Decision was wrong; they are now generated on
the runner itself, not in a container)

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
  > **Superseded.** "The same kind of container" was `rust:bookworm` against an
  > `ubuntu-latest` runner — Debian 12 vs Ubuntu 24.04, different font stacks. The
  > baselines never matched the runner. They are now rendered *on* the runner by
  > `.github/workflows/snapshot-baselines.yml`; see the amendment.
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
- Any snapshot-affecting UI change now needs **both** baselines refreshed: rerun
  locally for `-wgpu`, and dispatch the `snapshot-baselines` workflow for
  `-tiny-skia` (see the amendment below — do *not* generate the latter in a
  container).
- The coverage gate can go up over time as genuinely-testable gaps close (e.g.
  `phosphor::input`'s ~30%, a pure key→bytes function that's more testable than
  its number suggests) — it should track "what's realistic," not sit fixed at 60%
  forever.

## Amendment (2026-07-26) — the portability claim was wrong

The Decision above says the `tiny-skia` baselines were "verified byte-identical
across two independent fresh containers matching the `ubuntu-latest` CI runner",
and concludes the pixels are "portable and deterministic". **That did not hold,
and the verification was weaker than it sounds.**

Two containers agreeing with *each other* is not the same as either agreeing with
the runner. The baselines were generated in `rust:bookworm` — Debian 12 — while
CI runs on `ubuntu-latest`, which is Ubuntu 24.04. Different freetype/fontconfig
versions rasterize text differently, so the committed pixels never matched what
the runner produced.

This went unnoticed for two and a half weeks because **the snapshot step never
actually ran**. The same commit that introduced these baselines also left CI
failing earlier, on missing sibling checkouts (`dorado-engine`, `prexp-core`), and
every run after it failed the same way. The first time the step executed was
2026-07-26, at which point:

- **4 of the 5** committed `-tiny-skia` baselines did not match, and
- the other **61 snapshots had no `-tiny-skia` baseline at all**, so they were
  taking `matches_image`'s write-if-absent path and passing unconditionally. The
  step reported green while verifying essentially nothing.

### What replaces it

Stop trying to find an environment that matches the runner, and generate on the
runner. `.github/workflows/snapshot-baselines.yml` is dispatch-only: it clears the
`-tiny-skia` baselines, renders all of them on `ubuntu-latest`, and uploads them
as an artifact to commit. All 66 image-comparing snapshots now have a real
baseline, matching the `-wgpu` set name-for-name.

The original decision — force `tiny-skia` for CI, keep `wgpu` for local dev —
stands. Only the claim about where its baselines can be produced was wrong.

### The transferable lesson

The failure was not the wrong container; it was **a verification that could not
fail**. "Two containers agree" and "a missing baseline is written and passes" both
produce green without evidence. When a check's passing state is also its
no-op state, it is not a check. Prefer a signal that is loud when absent — which
is why the workflow now *clears* the baselines before rendering rather than
topping up whatever happens to be missing.
