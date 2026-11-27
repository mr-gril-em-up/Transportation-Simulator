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
# ACS 5-year ending in YEAR, every Texas block group (Census API JSON).
# Optional: without it zones keep default income and zero-car share.
# CENSUS_API_KEY is optional (keyless use is limited to 500 calls a day).
ACS_OUT="$RAW/acs5_bg48_${YEAR}.json"
ACS_Q="https://api.census.gov/data/${YEAR}/acs/acs5?get=B19013_001E,B25044_001E,B25044_003E,B25044_010E&for=block%20group:*"
ACS_KEY="${CENSUS_API_KEY:+&key=$CENSUS_API_KEY}"
is_acs() { [[ -s "$1" ]] && head -c 2 "$1" | grep -q '\[\['; }
if ! is_acs "$ACS_OUT"; then
  rm -f "$ACS_OUT"
  # One call for the whole state; the API may not accept a county wildcard.
  curl -sfL --retry 3 --retry-delay 10 -o "$ACS_OUT" "$ACS_Q&in=state:48&in=county:*&in=tract:*$ACS_KEY" || true
  if ! is_acs "$ACS_OUT"; then
    echo "ACS statewide call did not return JSON (first bytes below); fetching county by county" >&2
    head -c 300 "$ACS_OUT" >&2 2>/dev/null || true; echo >&2
    tmp=$(mktemp -d)
    for c in $(seq -f %03g 1 2 507); do  # Texas county FIPS codes are odd, 001-507
      curl -sfL --retry 3 --retry-delay 10 -o "$tmp/$c.json" "$ACS_Q&in=state:48%20county:$c$ACS_KEY" || true
      is_acs "$tmp/$c.json" || rm -f "$tmp/$c.json"
    done
    if ls "$tmp"/*.json >/dev/null 2>&1; then
      jq -s 'reduce .[] as $f ([]; if length == 0 then $f else . + $f[1:] end)' "$tmp"/*.json > "$ACS_OUT"
      echo "ACS: $(ls "$tmp"/*.json | wc -l) counties" >&2
    else
      rm -f "$ACS_OUT"; echo "warning: ACS download failed; building without it" >&2
    fi
    rm -rf "$tmp"
  fi
fi
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
