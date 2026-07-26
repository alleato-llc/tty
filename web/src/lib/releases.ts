// Build-time resolution of the download URLs, mirroring soroban's
// site/src/lib/releases.ts and dorado's web/src/lib/releases.ts so the three
// landing pages behave the same way.
//
// Why not just link `/releases/latest`? That resolves to the Releases *page*,
// so every visitor lands on a list of assets and has to pick one. Asking the
// Releases API at build time lets the buttons point straight at the file.
//
// tty is simpler than soroban here: ONE release track (`v*`) and no per-arch
// split — the macOS dmg is universal (Intel + Apple Silicon in one file) and
// Linux ships a single x86-64 archive. Windows is not built; see
// .github/workflows/release.yml's `build` job for why.
//
// Asset names carry the version (`Tty-0.1.0.dmg`), unlike soroban's stable
// version-free names, so these are matched by pattern rather than exact name.
// That keeps release.yml unchanged and works for every past and future tag.
//
// Runs in Astro frontmatter, i.e. at BUILD time (Node). One HTTP request; the
// `release: published` trigger on deploy-site.yml re-runs the build so the
// resolved URLs never go stale. On ANY failure (offline local build, rate
// limit, no release yet, missing asset) it falls back to a URL that always
// exists, so the site build can never break on this.

const REPO = "alleato-llc/tty";
const API = `https://api.github.com/repos/${REPO}/releases`;
const RELEASES_PAGE = `https://github.com/${REPO}/releases`;

export interface DownloadUrls {
  /** Universal macOS app — signed, notarized `Tty-<ver>.dmg`. */
  macDmg: string;
  /** Linux x86-64 archive — `tty-<ver>-x86_64-unknown-linux-gnu.tar.gz`. */
  linuxX64: string;
  /** Catch-all: the Releases page, used as the ultimate fallback. */
  releasesPage: string;
}

interface Asset {
  name: string;
  browser_download_url: string;
}
interface Release {
  tag_name: string;
  html_url: string;
  published_at: string;
  draft: boolean;
  prerelease: boolean;
  assets: Asset[];
}

async function fetchReleases(): Promise<Release[]> {
  const headers: Record<string, string> = {
    Accept: "application/vnd.github+json",
    "User-Agent": "tty-site-build",
  };
  // A token (present in CI) lifts the unauthenticated 60/hr rate limit.
  const token = process.env.GITHUB_TOKEN;
  if (token) headers.Authorization = `Bearer ${token}`;
  const res = await fetch(API, { headers });
  if (!res.ok) throw new Error(`GitHub Releases API ${res.status}`);
  return (await res.json()) as Release[];
}

/**
 * Newest published `v*` release. Drafts and prereleases are skipped so a
 * throwaway tag (`v0.0.1-rc1`) can be cut to exercise the pipeline without the
 * landing page pointing at it.
 */
function newest(releases: Release[]): Release | undefined {
  return releases
    .filter((r) => !r.draft && !r.prerelease && /^v\d/.test(r.tag_name))
    .sort((a, b) => Date.parse(b.published_at) - Date.parse(a.published_at))[0];
}

/** First asset URL matching any candidate, in order — else the release page. */
function pick(rel: Release | undefined, ...candidates: RegExp[]): string {
  for (const c of candidates) {
    const hit = rel?.assets.find((a) => c.test(a.name));
    if (hit) return hit.browser_download_url;
  }
  return rel?.html_url ?? RELEASES_PAGE;
}

export async function resolveDownloads(): Promise<DownloadUrls> {
  try {
    const rel = newest(await fetchReleases());
    return {
      macDmg: pick(rel, /^Tty-.*\.dmg$/i, /\.dmg$/i),
      linuxX64: pick(rel, /^tty-.*-x86_64-unknown-linux-gnu\.tar\.gz$/i, /linux.*\.tar\.gz$/i),
      releasesPage: RELEASES_PAGE,
    };
  } catch (err) {
    // Never fail the build on a download-link lookup — every URL degrades to
    // the Releases page, which always resolves.
    console.warn(`[releases] using Releases-page fallback: ${err}`);
    return {
      macDmg: RELEASES_PAGE,
      linuxX64: RELEASES_PAGE,
      releasesPage: RELEASES_PAGE,
    };
  }
}
