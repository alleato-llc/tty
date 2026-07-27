#!/usr/bin/env bash
# Mux the animated landing-page screenshots.
#
# `snapshot.rs::generate_landing_shots` renders the frames from the real app —
# `web/public/shots/_anim-<name>-f<n>[-theme].png`, gitignored — and this script
# assembles one animated WebP per theme, then deletes the frames:
#
#     web/public/shots/<name>[-theme].webp
#
# Run it after the generator, from anywhere:
#
#     cargo nextest run -p tty --ignore-default-filter --run-ignored all \
#         -E 'test(generate_landing_shots)'
#     web/scripts/build-shot-anims.sh
#
# Animated WebP rather than GIF: GIF is capped at 256 colours and bands visibly
# on the dark terminal gradients, and encodes several times larger. WebP keeps
# full colour and inter-frame compression makes the animation cost about the
# same as the single still it replaces.
#
# The static `<name>[-theme].png` shots stay — the page serves them to visitors
# who ask for reduced motion, since an animated image cannot be paused.
set -euo pipefail

SHOTS="$(cd "$(dirname "${BASH_SOURCE[0]}")/../public/shots" && pwd)"
command -v img2webp >/dev/null || {
  echo "img2webp not found — brew install webp" >&2
  exit 1
}

# Every animation the generator emits, discovered from the frame files so adding
# one on the Rust side needs no change here.
names=$(ls "$SHOTS" | sed -nE 's/^_anim-(.+)-f[0-9]+\.png$/\1/p' | sort -u)
[ -n "$names" ] || {
  echo "no _anim-*-f<n>.png frames in $SHOTS — run the generator first" >&2
  exit 1
}

# "" is the default (Dracula) capture; the rest match the page's data-theme values.
for name in $names; do
  for suffix in "" "-light" "-phosphor" "-github"; do
    frames=()
    n=0
    # `-f*` is greedy, so the unsuffixed glob also sweeps up the themed frames
    # (`-f0-light.png` matches `-f*`); the regex keeps only exact matches.
    for f in "$SHOTS/_anim-$name"-f*"$suffix.png"; do
      [[ "$f" =~ -f[0-9]+${suffix}\.png$ ]] || continue
      frames+=(-d 900 "$f")
      n=$((n + 1))
    done
    [ "$n" -gt 0 ] || continue
    out="$SHOTS/$name$suffix.webp"
    img2webp -loop 0 -lossy -q 88 "${frames[@]}" -o "$out" >/dev/null
    printf '  %-28s %2s frames  %6s KB\n' \
      "$(basename "$out")" "$n" "$(du -k "$out" | cut -f1)"
  done
done

rm -f "$SHOTS"/_anim-*.png
echo "frames cleaned up"
