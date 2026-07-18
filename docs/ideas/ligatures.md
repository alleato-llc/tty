# Programming ligatures (phosphor)

Status: **proposed, not committed.** A plan for optional cross-cell ligature rendering
(`!=` → `≠`, `=>` → `⇒`, `->`, `===`) in fonts that ship them (JetBrains Mono, Fira Code,
Cascadia Code). Opt-in via a setting.

## Why it doesn't work today

phosphor renders the grid **one glyph per cell**: for each maximal same-style run it still
calls `renderer.fill_text` once **per character**, each anchored to its own cell
(`content.x + col * cell_w`) — see `terminal.rs` draw, ~`:1077`. The shaper (`Shaping::Advanced`)
therefore only ever sees a **single character** string, so the OpenType `calt`/`liga`
substitutions that make ligatures never have the multi-char sequence to fire on. Font choice
can't change that.

The per-cell placement is deliberate. The comment at `:1048` explains: shaping a whole run and
letting the shaper's cumulative advance position glyphs makes them **drift off the monospace
grid** — invisible for a few chars, several cells' worth by column 20+. The likely root cause:
`cell_width()` measures `min_bounds` of `"M"` (`:1183`), and that rounded value doesn't exactly
equal the font's true per-glyph advance, so `col * cell_w` and the shaper's cumulative advance
diverge linearly.

## The core tension

Ligatures **require** shaping a run together; the monospace grid **requires** every cell to land
on `col * cell_w`. Real terminals (Kitty, WezTerm) resolve this by shaping runs and then
**snapping each glyph cluster back to the grid** — they need per-glyph/per-cluster placement.

phosphor is generic over `Renderer: text::Renderer<Font = Font>` (iced's abstraction), and
`fill_text` takes a `String` and renders it as a shaped paragraph **without exposing glyph
positions or cluster→source mapping**. So true cluster-snapping isn't available through the API
we use. That constraint drives the options below.

## Options

### A. Run-shaping with a calibrated cell width  ← recommended (pending a spike)

Render each eligible run as a **single `fill_text`** (content = the run's chars), and eliminate
the drift by setting `cell_w` to the font's **true advance** instead of the rounded `"M"`
measurement. Measure a long string (`"M"×64`) and divide by the count to get the exact advance;
use that everywhere `cell_w` is used.

Why it can hold the grid: in a real monospace ligature font every base glyph advances exactly one
cell and every ligature is designed to advance an **integer number of cells**, so a shaped run of
same-width glyphs lands on `col * cell_w` with no accumulated drift — *if* `cell_w` equals the
font's advance. Bonus: fewer `fill_text` calls than today (one per run, not per char).

**This hinges on a hypothesis** — that cosmic-text (through iced's `Paragraph`) positions a
monospace run's glyphs at exact advance multiples once `cell_w` is calibrated. **Spike first**
(see below). If it holds, this is a contained change entirely within `terminal.rs`, no renderer
coupling, works on both the wgpu and tiny-skia backends.

### B. Cluster placement via cosmic-text (fallback if A drifts)

Drop to the concrete wgpu renderer / cosmic-text to shape a run, read each glyph's cluster (byte
→ source cell), and place each cluster snapped to its first cell's `x`. Full control, exactly what
Kitty/WezTerm do. **Cost:** breaks phosphor's generic-`Renderer` design (it would need a concrete
path), and the tiny-skia test backend would fall back to per-cell (ligatures only on wgpu). Only
pursue if A can't hold alignment.

### C. Hardcoded ligature table — rejected

Detecting sequences without shaping (a per-font `calt` table) is fragile and font-specific
(JetBrains Mono has ~140). Not worth it.

## Segmentation: which runs get shaped

Ligature-shape only **maximal runs of cells that are all**: width-1 (not wide/CJK/emoji spacers),
same style key `(fg, bold, italic, underline)` (already the run boundary today), and **not broken
by** the rules below. Everything else renders per-cell exactly as now (the safe fallback stays the
default path).

Break a run (fall back to per-cell for those cells) at:

- **The cursor cell**, when the cursor is shown on this line — so the user sees the exact caret
  position; matches other terminals, which de-ligature under the cursor. Split the run around
  `screen.cursor_col`.
- **Wide glyphs** (`cell.width != 1`) — the width-0 spacer absorption assumes per-cell; mixing a
  CJK fallback face into a shaped run breaks advance-alignment. Break around them.
- **Inverse** already changes `fg` per cell (`cell_colors` swaps fg/bg), so inverse regions
  naturally break the run — no extra work.
- **Selection**: a separate bg tint drawn behind text, so it does *not* need to break ligatures
  (the tint reads fine behind a ligature). Leave it; revisit only if legibility complaints appear.
- **RTL / BiDi**: `Shaping::Advanced` can reorder a mixed run. The terminal grid is LTR; keep
  non-LTR runs on the per-cell path to avoid reordering (detect via a quick "any char above the
  BMP-Latin range needs the safe path" check, or a bidi class check).

## The setting (opt-in, off by default)

Ligatures are opt-in — they change rendering, carry the alignment risk on unusual fonts, and some
users dislike them. Off by default, matching tty's opt-in ethos (history, env, notifications).

- **phosphor** stays theme-crate-free: add a plain `ligatures: bool` to the `Terminal` widget
  builder (alongside `font` / `font_size`), defaulting false. No behavior unless the host sets it.
- **tty** adds `Settings.terminal_ligatures: Option<bool>` and passes it into the widget; a toggle
  in **Appearance → Terminal**. Wiring the toggle is the `ui-settings` skill; the round-trip,
  resolver default, and the control are standard.
- **fed-ide** can opt in the same way (it constructs the widget from its own settings).
- Live: the setting is read at view time, so flipping it re-renders immediately (no restart).

## Testing

- **Unit-test the segmentation** — the pure function that splits a line's cells into
  (shaped-run | per-cell) spans given cursor col, widths, style keys, and the ligature flag. This
  is the load-bearing logic and needs no real shaper. Sibling `terminal_tests.rs`.
- **Alignment regression** — a snapshot with a **normal** font must be byte-identical before/after
  the `cell_w` calibration change (the calibration must not move anything on non-ligature text).
  Watch the existing terminal snapshots; re-baseline only if a sub-pixel shift is expected and
  justified.
- **Ligature snapshot** — fragile, because iced loads a font family by name and a machine without
  JetBrains Mono silently falls back (no ligatures, non-deterministic). To make it deterministic,
  **embed a small ligature test font** in the test and load it via `.font(bytes)`; otherwise this
  stays manual verification (type `www -> => != <= === |>` and eyeball). Prefer the embedded-font
  route so CI can assert it.

## Risks

- **The spike may fail** — if calibrated run-shaping still drifts, Option A is out and B is a much
  bigger change; decide then whether ligatures are worth the renderer coupling.
- **Font-dependent alignment** — a "monospace" font whose ligatures aren't integer-cell-width would
  drift under A. Mitigate: the setting is opt-in, and we can keep a per-run width sanity check
  (if a shaped run's measured width != `span * cell_w` beyond a tolerance, fall back to per-cell for
  that run).
- **Shared widget** — phosphor backs fed-ide too; keep the default path (per-cell) byte-identical so
  nothing changes for hosts that don't opt in.

## Phased plan

1. **Spike (½ day):** calibrate `cell_w` to the true advance; render one shaped run at a known
   style; measure glyph x at col 0/40/80 vs `col * cell_w`. Decide A vs B. *Gate the whole effort
   on this.*
2. **Segmentation + render (A):** the pure line-segmentation fn + its unit tests; route shaped
   spans through one `fill_text`, per-cell spans unchanged; break at cursor/wide/RTL.
3. **Setting:** the phosphor `ligatures` flag + tty `Settings` field + the Appearance→Terminal
   toggle (`ui-settings`); fed-ide wiring optional.
4. **Tests + docs:** segmentation unit tests, the alignment-regression snapshot, an embedded-font
   ligature snapshot if feasible; note the feature in README + this file's status.
