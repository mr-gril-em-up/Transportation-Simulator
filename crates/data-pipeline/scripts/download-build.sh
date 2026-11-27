#!/usr/bin/env bash
# Downloads a published pipeline build (GitHub Release "data-<area>-<year>[-<level>]")
# into data/out/<area>-<year>[-<level>] and prints the TRANSPO_SCENARIO line for the app.
#   ./scripts/download-build.sh               # Travis County, LODES 2023
#   AREA=48 YEAR=2023 ./scripts/download-build.sh
#   AREA=48 LEVEL=tract ./scripts/download-build.sh   # statewide, tract zones
#   AREA=48 LEVEL=tract ROADS=highway ./scripts/download-build.sh   # strategic tier (DATA-8)
set -euo pipefail
cd "$(dirname "$0")/.."
AREA="${AREA:-48453}"
YEAR="${YEAR:-2023}"
LEVEL="${LEVEL:-bg}"
SUFFIX=""
[ "$LEVEL" = bg ] || SUFFIX="-$LEVEL"
[ "${ROADS:-all}" = all ] || SUFFIX="$SUFFIX-$ROADS"
REPO="${REPO:-mr-gril-em-up/Transportation-Simulator}"
OUT="data/out/$AREA-$YEAR$SUFFIX"
URL="https://github.com/$REPO/releases/download/data-$AREA-$YEAR$SUFFIX/tsim-data-$AREA-$YEAR$SUFFIX.tar.gz"
mkdir -p "$OUT"
echo "get  $URL" >&2
curl -fL --retry 3 "$URL" | tar -xz -C "$OUT"
echo "export TRANSPO_SCENARIO=$(pwd)/$OUT/scenario.json"
