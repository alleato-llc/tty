# tty web

The landing page that advertises **tty**. A static [Astro](https://astro.build/)
site with a Preact theme toggle — the same setup as the sibling `fed` project's
site. It's a single-page **slide deck** (the "stepper") — arrow keys, on-screen
arrows, or the dots step through the pitch.

## Develop

```
npm install
npm run dev      # local dev server
npm run build    # static output to dist/
npm run preview  # serve the built dist/
```

## Layout

Pages share one shell:

- `src/pages/` — one `.astro` file per route (`index.astro` is the deck).
- `src/layouts/Layout.astro` — the shared shell: `<head>`, the theme bootstrap
  script, the fixed chrome (wordmark + theme toggle), and a `<slot />`.
- `src/components/ThemeToggle.tsx` — the one interactive island (Preact).
- `src/styles/global.css` — the two-theme design system via the `data-theme`
  attribute. **Dark is tty's Dracula; light is Solarized Light.** The page chrome
  draws from these tokens, so the toggle re-skins it.
- `public/shots/` — the terminal screenshots (see below).
- `public/` — other static assets served as-is (favicon).

The deck markup lives in one raw-HTML string in `index.astro` injected via
`set:html` (a template literal, so the literal `{ }` in commands aren't parsed as
JSX). Each terminal shown is a **real screenshot of the app** (`public/shots/*`),
never a mockup.

## Screenshots

The shots in `public/shots/` are rendered by the *actual* app — the same headless
wgpu path the snapshot tests use — not hand-drawn. Regenerate them from the tty
crate:

```
cd ../tty   # the app crate
cargo nextest run -p tty --ignore-default-filter --run-ignored all \
  -E 'test(generate_landing_shots)'
```

The generator (`tty/src/snapshot.rs::generate_landing_shots`, `#[ignore]`d) writes
each shot straight into `web/public/shots/`. To restyle content or add a shot, edit
that function, rerun, and (optionally) `sips -Z 1400 web/public/shots/*.png` to keep
the web assets light.

Every shot is rendered in **all four themes the page can wear** —
`<name>.png` (Dracula), `-light` (Solarized Light), `-phosphor`, `-github` — because
the theme-grid easter egg on slide 7 re-skins the page and the screenshots follow.
Only one is ever downloaded: the page keeps a single `<img>` per shot and rewrites
its `src` (`SHOT_SUFFIX` in `index.astro`), rather than hiding copies in the DOM,
which browsers fetch anyway.

### Animated shots

The status-bar shot is animated — a cell drills in and is dragged into place. The
generator renders the frames; a second step muxes them:

```
cd ../tty && cargo nextest run -p tty --ignore-default-filter --run-ignored all \
  -E 'test(generate_landing_shots)'
cd .. && web/scripts/build-shot-anims.sh      # needs `brew install webp`
```

The frames land as `public/shots/_anim-<name>-f<n>[-theme].png` (gitignored) and the
script turns them into `<name>[-theme].webp`, then deletes them. **Both steps are
needed** — running only the generator leaves the old `.webp` in place next to fresh
frames.

Animated WebP rather than GIF: GIF caps at 256 colours and bands visibly on the dark
terminal gradients, at several times the size. The still `.png` is kept and served to
anyone who asks for reduced motion, via a `<picture>` media query — an animated image
cannot be paused.

## Downloads

The Download buttons link to `…/releases/latest` on GitHub, where
`.github/workflows/release.yml` attaches the prebuilt apps: a **signed, notarized,
universal `.dmg`** for macOS (built via `salpa` — see `ci/salpa-tty.yaml`), and
`tar.gz` / `zip` archives for Linux and Windows. The `REPO` constant at the top
of `src/pages/index.astro` points at `alleato-llc/tty`.

## Deploy

`salpa deploy` builds this dir and syncs `dist/` to S3 + CloudFront
(`tty.alleato.dev`); see `../salpa.yaml` and `.github/workflows/deploy-site.yml`.
The deploy runs automatically on pushes to `main` that touch `web/`.
