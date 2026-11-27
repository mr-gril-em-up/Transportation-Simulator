#!/usr/bin/env bash
# Downloads the raw inputs for tsim-data into data/raw (about 800 MB, mostly OSM).
# The same files cover all of Texas; --area picks Travis or the whole state at build time.
#   YEAR=2023 ./scripts/fetch.sh            # all inputs
#   SKIP_OSM=1 ./scripts/fetch.sh           # census only (zones + flows, no roads)
set -euo pipefail
cd "$(dirname "$0")/.."
YEAR="${YEAR:-2023}"
RAW=data/raw
mkdir -p "$RAW"

get() { # get NAME URL [FALLBACK_URL]: saves data/raw/NAME, skipped if present
  local out="$RAW/$1"; shift
  if [[ -s "$out" ]]; then echo "have $out"; return; fi
  for url in "$@"; do
    echo "get  $url"
    if curl -fL --retry 5 --retry-all-errors --retry-delay 15 -o "$out.part" "$url"; then mv "$out.part" "$out"; return; fi
  done
  echo "FAILED $out" >&2; return 1
}

# Block-group population and population-weighted centroids (2020 Census).
get CenPop2020_Mean_BG48.txt https://www2.census.gov/geo/docs/reference/cenpop2020/blkgrp/CenPop2020_Mean_BG48.txt
# LODES 8 (2020 blocks): home->work flows, workers by home, jobs by workplace.
LODES=https://lehd.ces.census.gov/data/lodes/LODES8/tx
# The LODES 8.4 tech doc shows RAC names both with and without a "_1" suffix; try both.
get "tx_od_main_JT00_${YEAR}.csv.gz" "$LODES/od/tx_od_main_JT00_${YEAR}.csv.gz"
get "tx_rac_S000_JT00_${YEAR}.csv.gz" "$LODES/rac/tx_rac_S000_JT00_${YEAR}.csv.gz" "$LODES/rac/tx_rac_S000_JT00_${YEAR}_1.csv.gz"
get "tx_wac_S000_JT00_${YEAR}.csv.gz" "$LODES/wac/tx_wac_S000_JT00_${YEAR}.csv.gz"
# OpenStreetMap Texas extract (ODbL, attribution required).
[[ -n "${SKIP_OSM:-}" ]] || get texas-latest.osm.pbf https://download.geofabrik.de/north-america/us/texas-latest.osm.pbf
