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
  attribute. **Dark is tty's Dracula; light is GitHub Light.** Everything,
  including the terminal mockups, draws from these tokens, so the toggle re-skins
  the whole page the way switching themes re-skins the app.
- `public/` — static assets served as-is (favicon).

The deck markup lives in one raw-HTML string in `index.astro` injected via
`set:html`, because the terminal mockups are full of literal `{ }` that Astro
would otherwise parse as JSX expressions. The terminal panes are CSS mockups,
not screenshots — drop in real captures later by replacing the `.win` blocks.

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
