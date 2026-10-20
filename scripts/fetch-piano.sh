#!/bin/sh
# Downloads the Salamander Grand Piano samples (Alexander Holm, CC BY 3.0) used by the in-browser player.
# Mirrored by the Tone.js project. ~30 mp3 files, a few MB in total. Safe to re-run.
set -e
cd "$(dirname "$0")/.."
DEST=frontend/public/samples/piano
mkdir -p "$DEST"
BASE=https://tonejs.github.io/audio/salamander
NAMES="A0 C8"
for o in 1 2 3 4 5 6 7; do NAMES="$NAMES A$o C$o Ds$o Fs$o"; done
for n in $NAMES; do
  [ -s "$DEST/$n.mp3" ] || curl -fsSL "$BASE/$n.mp3" -o "$DEST/$n.mp3"
done
echo "piano samples ready in $DEST ($(ls $DEST | wc -l) files, $(du -sh $DEST | cut -f1))"
