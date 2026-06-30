// @ts-check
import { defineConfig } from "astro/config";
import preact from "@astrojs/preact";

// Static marketing site for fed. Preact powers the single interactive island
// (the light/dark theme toggle); everything else is static Astro plus a small
// inline deck script. Flat-file output so extensionless URLs resolve cleanly on
// static hosts that append `.html`.
export default defineConfig({
  integrations: [preact()],
  build: { format: "file" },
});
