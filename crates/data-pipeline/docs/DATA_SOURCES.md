# Data sources

Every input the pipeline reads, where it comes from, what it gives the game, and its limits. All are free and public. `scripts/fetch.sh` downloads them into `data/raw/` (gitignored). Raw data is never committed.

URLs and formats below were checked against the publishers' pages on 2026-11-27. The cloud environment that wrote this code could not reach the download hosts, so the first real download (run `fetch.sh` locally or the Actions workflow) is the end-to-end confirmation.

## 1. Census 2020 Centers of Population, block groups

| | |
|---|---|
| Publisher | US Census Bureau, Geography Division |
| Page | https://www.census.gov/geographies/reference-files/2020/geo/2020-centers-population.html |
| File | https://www2.census.gov/geo/docs/reference/cenpop2020/blkgrp/CenPop2020_Mean_BG48.txt (48 = Texas FIPS) |
| Vintage | 2020 Decennial Census |
| Size | About 1 MB for Texas |
| Columns used | `STATEFP, COUNTYFP, TRACTCE, BLKGRPCE, POPULATION, LATITUDE, LONGITUDE` |
| License | US federal government work, public domain |
| Gives the game | The zone list (one zone per block group), each zone's population, and its population-weighted centroid |

Why this file and not TIGER polygons: it gives population and a population-weighted point in one small CSV, so the pipeline needs no shapefile or geometry library. Polygons are only needed for map fills and will be added as a separate, optional layer.

The reader accepts the file with or without a header row and strips a UTF-8 byte-order mark.

**Citation:** U.S. Census Bureau, "Centers of Population for the 2020 Census," block group file for Texas.

## 2. LEHD Origin-Destination Employment Statistics (LODES) 8

| | |
|---|---|
| Publisher | US Census Bureau, Longitudinal Employer-Household Dynamics (LEHD) program |
| Data root | https://lehd.ces.census.gov/data/lodes/LODES8/tx/ |
| Technical doc | https://lehd.ces.census.gov/data/lodes/LODES8/LODESTechDoc8.4.pdf (Format Version 8.4, rev. 2025-12-03) |
| Release note | https://www.census.gov/programs-surveys/ces/news-and-updates/updates/12182025.html (LODES 8.4 adds 2023; covers 2002–2023) |
| Geography | 2020 census blocks (15-digit GEOID) |
| Default year | 2023 (`--year` / `YEAR=` to change) |
| License | US federal government work, public domain; LODES applies noise for confidentiality |

Files read (job type `JT00` = all jobs):

| File | Path | Columns used | Gives |
|---|---|---|---|
| `tx_od_main_JT00_<year>.csv.gz` | `od/` | `w_geocode, h_geocode, S000` | Workers per (home block, work block) pair, both in Texas |
| `tx_rac_S000_JT00_<year>.csv.gz` | `rac/` | `h_geocode, C000` | Workers living in each block |
| `tx_wac_S000_JT00_<year>.csv.gz` | `wac/` | `w_geocode, C000` | Jobs located in each block |

The 8.4 tech doc's RAC filename pattern shows a trailing `_1` (`[ST]_rac_[SEG]_[TYPE]_[YEAR]_1.csv.gz`) while its example omits it. `fetch.sh` tries both and saves under the name without `_1`.

Known gaps (from the tech doc and the design doc's risk list):

- Covers jobs in Unemployment Insurance records plus federal civilian jobs. Self-employed workers, most military, and some other categories are missing, so LODES undercounts total workers.
- Counts are jobs, not people: a person with two jobs appears twice in RAC and OD.
- Values are noise-infused for confidentiality; small block-level counts are unreliable, which is one reason the pipeline aggregates to block groups.
- Lags 2–3 years behind the present.
- `od_main` holds only Texas residents working in Texas. Out-of-state residents working in Texas are in `od_aux` and are not loaded yet.

**Citation:** U.S. Census Bureau, LEHD Origin-Destination Employment Statistics (LODES) Version 8.4, Texas, 2023. Longitudinal Employer-Household Dynamics Program.

## 3. OpenStreetMap, Geofabrik Texas extract

| | |
|---|---|
| Publisher | OpenStreetMap contributors; extract by Geofabrik GmbH |
| File | https://download.geofabrik.de/north-america/us/texas-latest.osm.pbf |
| Vintage | Daily snapshot; record the download date when publishing a build |
| Size | Several hundred MB |
| License | Open Database License (ODbL) 1.0 |
| Gives the game | Drivable roads (class, speed, lanes, one-way, name, geometry); existing transit routes and stops |

Tags read: `highway`, `maxspeed`, `lanes`, `oneway`, `junction`, `area`, `name` on ways; `type=route`, `route`, `ref`, `name`, `operator` and member roles on relations; `name` on stop nodes.

**License obligations:** the derived road database must credit "© OpenStreetMap contributors" wherever it is shown, and a database derived from OSM and distributed publicly must be offered under ODbL. The design doc flags a legal read before any commercial sale.

**Citation:** OpenStreetMap contributors (2026). Texas extract retrieved from Geofabrik, https://download.geofabrik.de. Data available under the Open Database License.

## 4. American Community Survey 5-year, block groups

| | |
|---|---|
| Publisher | US Census Bureau |
| Files | `https://www2.census.gov/programs-surveys/acs/summary_file/<year>/table-based-SF/data/5YRData/acsdt5y<year>-b19013.dat` and `...-b25044.dat` (pipe-delimited, every US geography; `fetch.sh` keeps Texas block groups) |
| Vintage | 5-year estimates ending in the LODES year (2019–2023 by default), 2020 block groups |
| Tables | B19013 median household income (`B19013_E001`); B25044 tenure by vehicles available (`B25044_E001` households, `_E003` owner no vehicle, `_E010` renter no vehicle) |
| Key | None. (The Census API was tried first and refused keyless requests with "Missing Key".) |
| License | US federal government work, public domain |
| Gives the game | `median_income`, `zero_car_share` per zone |

Margins of error are large at block-group level, which is why suppressed values fall back to tract and county means rather than to a statewide default.

**Citation:** U.S. Census Bureau, American Community Survey 5-Year Estimates, Tables B19013 and B25044, Texas block groups.

## 5. TIGER/Line 2020 block groups

| | |
|---|---|
| Publisher | US Census Bureau, Geography Division |
| File | https://www2.census.gov/geo/tiger/TIGER2020/BG/tl_2020_48_bg.zip (only the `.dbf` is kept) |
| Columns used | `GEOID`, `ALAND` (land area, m²) |
| License | US federal government work, public domain |
| Gives the game | `area_km2` per zone. Polygons (map fills) are not read yet |

**Citation:** U.S. Census Bureau, 2020 TIGER/Line Shapefiles, Block Groups, Texas.

## Planned sources (not read yet)

From the overview doc's data table, in the order the pipeline will likely add them:

| Source | For | Milestone |
|---|---|---|
| ACS 5-year commute mode share (B08301) | Mode-choice calibration | M1 |
| TIGER/Line block group polygons | Zone polygons for map fills | M1 |
| CapMetro GTFS | Red Line schedule for the M1 ridership check | M1 |
| ACS PUMS | Synthetic population | M2+ |
| NHTS | Non-work trip rates | M2+ |
| TxDOT AADT counts | Road assignment calibration | M3 |
| USGS 3DEP, NLCD | Terrain and land-cover cost multipliers | M3 |
