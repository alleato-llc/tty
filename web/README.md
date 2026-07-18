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
  attribute. **Dark is tty's Dracula; light is GitHub Light.** The page chrome
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
