#!/usr/bin/env bash
# Downloads the raw inputs for tsim-data into data/raw (about 800 MB, mostly OSM).
# The same files cover all of Texas; --area picks Travis or the whole state at build time.
#   YEAR=2023 ./scripts/fetch.sh            # all inputs
#   SKIP_OSM=1 ./scripts/fetch.sh           # census only (zones, flows, ACS, area; no roads)
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
# ACS 5-year ending in YEAR: table-based summary files (one pipe-delimited
# file per table, every US geography; no API key needed). Only the header and
# Texas block-group rows (GEO_ID 1500000US48...) are kept, plus Texas county
# rows (0500000US48...) so published county totals can be checked against the
# block-group sums. B08301 (commute mode) feeds the manifest's mode shares.
# Optional: without them zones keep default income and zero-car share.
ACS_SF="https://www2.census.gov/programs-surveys/acs/summary_file/${YEAR}/table-based-SF/data/5YRData"
for t in b19013 b25044 b08301; do
  out="$RAW/acs5_bg48_${YEAR}_${t}.dat"
  if [[ -s "$out" ]]; then echo "have $out"; continue; fi
  echo "get  $ACS_SF/acsdt5y${YEAR}-${t}.dat"
  if curl -fsSL --retry 5 --retry-all-errors --retry-delay 15 "$ACS_SF/acsdt5y${YEAR}-${t}.dat" \
       | awk -F'|' 'NR == 1 || $1 ~ /^(1500000|0500000)US48/' > "$out.part" && [[ $(wc -l < "$out.part") -gt 1 ]]; then
    mv "$out.part" "$out"; echo "     $(grep -c '^1500000US' "$out") Texas block groups"
    if [[ $t == b08301 ]]; then head -1 "$out"; grep '^0500000US48453|' "$out" || true; fi
  else
    rm -f "$out.part"; echo "warning: ACS table $t download failed; building without ACS" >&2
  fi
done
# TIGER/Line 2020 block groups: only the .dbf attribute table (ALAND) is kept.
if [[ ! -s "$RAW/tl_2020_48_bg.dbf" ]]; then
  if get tl_2020_48_bg.zip https://www2.census.gov/geo/tiger/TIGER2020/BG/tl_2020_48_bg.zip; then
    unzip -p "$RAW/tl_2020_48_bg.zip" tl_2020_48_bg.dbf > "$RAW/tl_2020_48_bg.dbf" && rm "$RAW/tl_2020_48_bg.zip"
  else
    echo "warning: TIGER download failed; zones will have no land area" >&2
  fi
fi
# GTFS static feeds, one per agency in reference/gtfs_feeds.csv (headways and
# stops for today's network, DATA-6/DATA-9). Optional, small; a feed that
# fails to download only drops that agency's lines.
if [[ -z "${SKIP_OSM:-}" ]]; then
  while IFS=, read -r id agency url fallback _; do
    dir="$RAW/gtfs/$id"
    if [[ -s "$dir/routes.txt" ]]; then echo "have $dir"; continue; fi
    if get "gtfs_$id.zip" "$url" ${fallback:+"$fallback"} \
       && { unzip -o -q -j "$RAW/gtfs_$id.zip" '*routes.txt' '*trips.txt' '*stop_times.txt' '*stops.txt' '*calendar*.txt' -d "$dir" \
            || [[ -s "$dir/routes.txt" && -s "$dir/stop_times.txt" ]]; }; then
      echo "     $agency: $(($(wc -l < "$dir/trips.txt") - 1)) GTFS trips"
    else
      echo "warning: $agency GTFS download failed; its routes have no headways" >&2
    fi
  done < <(tail -n +2 reference/gtfs_feeds.csv)
fi
# OpenStreetMap Texas extract (ODbL, attribution required).
[[ -n "${SKIP_OSM:-}" ]] || get texas-latest.osm.pbf https://download.geofabrik.de/north-america/us/texas-latest.osm.pbf
