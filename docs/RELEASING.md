# Releasing tty

Releases are driven by a **version tag**, not by merges. Pushing `v*` runs
`.github/workflows/release.yml`, which builds and publishes everything onto one
GitHub Release using [salpa](https://github.com/alleato-llc/salpa) (our house
release tool, pulled from ghcr as a private OCI artifact).

```sh
git tag v0.1.0 && git push origin v0.1.0
```

This differs from dorado, which computes the next semver from commit messages and
releases on merge to `main`. tty has one product and one track, so an explicit tag
is simpler and means a merge can never publish something by surprise.

## What ships

Everything lands on the same `v<version>` GitHub Release:

- **macOS** — a **signed, notarized, universal** `Tty-<version>.dmg`. One dmg runs
  on both Apple Silicon and Intel (cargo builds both arches, `lipo` merges them).
  tty ships as `Tty.app`, not a bare `tty` binary on `PATH`, so it never shadows
  the POSIX `tty(1)` command.
- **Linux** — a bare, unsigned `tty-<version>-x86_64-unknown-linux-gnu.tar.gz`
  containing the binary plus `README.md` and `LICENSE`.

**Windows is not built.** `tty/src/metrics.rs` imports
`prexp_core::backend::NativeSource`, which `prexp-core` only defines for macOS and
Linux — its `backend/mod.rs` has `#[cfg(target_os = "macos")]` and
`#[cfg(target_os = "linux")]` arms and no Windows one — so the binary fails to
link with `E0432`/`E0433`. To restore it, give `prexp-core` a Windows backend (or
`cfg`-gate `metrics.rs` to a no-op sampler there), then re-add the matrix entry
documented in `release.yml`'s `build` job. The `Package` step still handles
`ext: zip`, so it is a three-line change.

## Pipeline shape

```
test  →  create-release  →  build        (Linux tar.gz, unsigned)
                         →  build-macos  (universal dmg, SIGNED + NOTARIZED)
```

`test` is a real gate: tag pushes bypass branch protection, so the release runs the
suite itself and nothing is created or published if it fails. `create-release` cuts
the empty release once so the two parallel build legs upload into it
deterministically.

Every Rust job checks out **three sibling path dependencies** next to tty —
`rime`, `dorado`, and `prexp` (into a directory named `fdtop`, which is what
`Cargo.toml:57` expects). Miss one and `cargo metadata` fails before anything
compiles. See `.github/workflows/ci.yml` for the full note.

## One-time setup: the five secrets (macOS signing)

Needed once, in **this repo's** GitHub settings — **Settings → Secrets and
variables → Actions → New repository secret**. This is a fresh setup even if you
already have these secrets configured for another repo (e.g. `dorado`, `soroban`):
GitHub secrets are per-repository, so the same underlying Apple Developer ID
certificate still needs to be re-exported and re-added here.

| Secret | Value |
|---|---|
| `BUILD_CERTIFICATE_BASE64` | your Developer ID Application certificate **with its private key**, exported as `.p12`, base64-encoded |
| `P12_PASSWORD` | the password you chose during the `.p12` export |
| `APPLE_TEAM_ID` | the 10-character team id (developer.apple.com → Membership) |
| `APPLE_ID` | the Apple ID email used for notarization |
| `APPLE_APP_SPECIFIC_PASSWORD` | an app-specific password — create at [appleid.apple.com](https://appleid.apple.com) → Sign-In and Security → App-Specific Passwords |

Until these exist, `build-macos` fails at the signing step. The Linux archive is
unaffected — it needs no secrets.

Note that `gh secret list` only proves a secret **exists**, not that its value is
right. A truncated base64 blob or a mistyped app-specific password looks identical
to a correct one until the release actually runs.

### Exporting the certificate

You need a **Developer ID Application** certificate (not "Apple Development" /
"Mac App Distribution"). If you don't have one yet: Xcode → Settings → Accounts →
Manage Certificates → + → Developer ID Application (or developer.apple.com →
Certificates). This requires an active Apple Developer Program membership.

1. Open **Keychain Access** → My Certificates.
2. Find "Developer ID Application: Your Name (TEAMID)" — expand it and confirm the
   private key is underneath. **No key means signing will fail**: the `.p12`
   imports fine and then `codesign` errors out. Export from the Mac that created
   the certificate.
3. Right-click the certificate → **Export…** → format `.p12`, choose a password
   (that's `P12_PASSWORD`).
4. Base64 it onto the clipboard and paste into the secret:

   ```sh
   base64 -i Certificates.p12 | pbcopy
   ```

### Pulling salpa

The workflow pulls the `salpa` binary from ghcr (`ghcr.io/alleato-llc/salpa`) via
`oras`, authenticated with the workflow's own `GITHUB_TOKEN` (`packages: read`).
The version is **pinned** in `release.yml`'s `env` block (`SALPA_VERSION`) — bump
it deliberately. It used to be `go install …@latest`, which was both unpinned and
broken (salpa's module is private), so a salpa release could break the pipeline
unannounced.

`build-macos` pulls the **`salpa-darwin-arm64`** artifact (`macos-latest` is Apple
Silicon) into `RUNNER_TEMP` and puts it on `PATH`, overriding the job's
`working-directory: tty` so the build still runs from the workspace root.

## Config

Two salpa configs, one product each — salpa is one product per config:

| File | Deliverable |
|---|---|
| `ci/salpa-tty.yaml` | the signed macOS dmg (`bundle_id: dev.tty.terminal`, universal, notarize, staple) |
| `salpa.yaml` (root) | the landing page deploy — a different workflow entirely (`deploy-site.yml`) |

## Day-to-day

```sh
git checkout -b feature/thing     # ci.yml runs fmt, clippy, tests, snapshots, coverage
…                                 # open a PR, merge to main
git tag v0.2.0 && git push origin v0.2.0
```

- **A failed release** (a notarization hiccup, a missing secret): fix the cause and
  **re-run the workflow run** rather than cutting a new tag — the commit is already
  tagged, so it rebuilds the same version.
- **First time exercising a change to the release path**, burn a throwaway
  prerelease tag (`v0.0.1-rc1`) rather than a real version, then delete it:

  ```sh
  gh release delete v0.0.1-rc1 --yes --cleanup-tag
  ```

  The landing page's download resolver filters prereleases, so an rc tag never
  becomes the advertised download.

## Verifying a release

A green checkmark is not proof the artifact is good — signing can succeed with the
wrong identity, and stapling can be skipped silently. Check the artifact:

```sh
gh release download v0.1.0 -p 'Tty-*.dmg'

spctl -a -vvv -t open --context context:primary-signature Tty-0.1.0.dmg
#   → accepted / source=Notarized Developer ID
xcrun stapler validate Tty-0.1.0.dmg
#   → The validate action worked!

hdiutil attach Tty-0.1.0.dmg -nobrowse
lipo -archs /Volumes/Tty/Tty.app/Contents/MacOS/tty     # → x86_64 arm64
codesign -dv --verbose=2 /Volumes/Tty/Tty.app           # → flags=0x10000(runtime)
hdiutil detach /Volumes/Tty
```

Expect `Authority=Developer ID Application: …`, `Identifier=dev.tty.terminal`, and
the `runtime` flag (hardened runtime).

One nuance: the ticket is stapled to the **dmg**, not to `Tty.app` inside it, so
`stapler validate` on the app reports no ticket. That is normal for dmg
distribution and Gatekeeper accepts it — but someone who copies the app out and
first launches it offline will hit an online-verification delay.

## The landing page

`web/src/lib/releases.ts` resolves the download URLs from the Releases API **at
build time**, so the buttons link straight at the assets. A `release: published`
trigger on `deploy-site.yml` redeploys the site whenever a release is cut —
without it the resolved URLs go stale the moment you tag. Nothing to do manually.

## Icons

`tty/assets/icon.svg` is the master. `AppIcon.icns` (10 sizes) is generated from
it and named by `ci/salpa-tty.yaml`'s `icon:` field for the `.app` bundle;
`icon-512.png` is embedded in the binary for the Linux/Windows taskbar and the
runtime macOS Dock icon (`tty/src/app_icon.rs`). To restyle: edit the SVG,
re-render to a 1024 PNG, then `sips` an `.iconset` and `iconutil -c icns`, and
refresh `icon-512.png`.
