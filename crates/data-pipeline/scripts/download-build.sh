#!/usr/bin/env bash
# Downloads a published pipeline build (GitHub Release "data-<area>-<year>")
# into data/out/<area>-<year> and prints the TRANSPO_SCENARIO line for the app.
#   ./scripts/download-build.sh               # Travis County, LODES 2023
#   AREA=48 YEAR=2023 ./scripts/download-build.sh
set -euo pipefail
cd "$(dirname "$0")/.."
AREA="${AREA:-48453}"
YEAR="${YEAR:-2023}"
REPO="${REPO:-mr-gril-em-up/Transportation-Simulator}"
OUT="data/out/$AREA-$YEAR"
URL="https://github.com/$REPO/releases/download/data-$AREA-$YEAR/tsim-data-$AREA-$YEAR.tar.gz"
mkdir -p "$OUT"
echo "get  $URL" >&2
curl -fL --retry 3 "$URL" | tar -xz -C "$OUT"
echo "export TRANSPO_SCENARIO=$(pwd)/$OUT/scenario.json"
