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
# ACS 5-year ending in YEAR, every Texas block group, one Census API call.
# Optional: without it zones keep default income and zero-car share.
# CENSUS_API_KEY is optional (keyless use is limited to 500 calls a day).
ACS="https://api.census.gov/data/${YEAR}/acs/acs5?get=B19013_001E,B25044_001E,B25044_003E,B25044_010E&for=block%20group:*&in=state:48&in=county:*&in=tract:*${CENSUS_API_KEY:+&key=$CENSUS_API_KEY}"
get "acs5_bg48_${YEAR}.json" "$ACS" || echo "warning: ACS download failed; building without it" >&2
# TIGER/Line 2020 block groups: only the .dbf attribute table (ALAND) is kept.
if [[ ! -s "$RAW/tl_2020_48_bg.dbf" ]]; then
  if get tl_2020_48_bg.zip https://www2.census.gov/geo/tiger/TIGER2020/BG/tl_2020_48_bg.zip; then
    unzip -p "$RAW/tl_2020_48_bg.zip" tl_2020_48_bg.dbf > "$RAW/tl_2020_48_bg.dbf" && rm "$RAW/tl_2020_48_bg.zip"
  else
    echo "warning: TIGER download failed; zones will have no land area" >&2
  fi
fi
# OpenStreetMap Texas extract (ODbL, attribution required).
[[ -n "${SKIP_OSM:-}" ]] || get texas-latest.osm.pbf https://download.geofabrik.de/north-america/us/texas-latest.osm.pbf
