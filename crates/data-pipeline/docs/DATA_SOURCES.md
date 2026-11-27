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
| Tables | B19013 median household income (`B19013_E001`); B25044 tenure by vehicles available (`B25044_E001` households, `_E003` owner no vehicle, `_E010` renter no vehicle); B08301 means of transportation to work (`_E001` workers 16+, `_E003` drove alone, `_E004` carpool, `_E010` public transportation excluding taxicab, `_E016` taxicab, `_E017` motorcycle, `_E018` bicycle, `_E019` walked, `_E020` other, `_E021` worked from home). Texas county rows are kept too, so county totals can be checked |
| Key | None. (The Census API was tried first and refused keyless requests with "Missing Key".) |
| License | US federal government work, public domain |
| Gives the game | `median_income`, `zero_car_share` per zone |

Margins of error are large at block-group level, which is why suppressed values fall back to tract and county means rather than to a statewide default.

**Citation:** U.S. Census Bureau, American Community Survey 5-Year Estimates, Tables B19013, B25044 and B08301, Texas block groups and counties.

## 5. TIGER/Line 2020 block groups

| | |
|---|---|
| Publisher | US Census Bureau, Geography Division |
| File | https://www2.census.gov/geo/tiger/TIGER2020/BG/tl_2020_48_bg.zip (only the `.dbf` is kept) |
| Columns used | `GEOID`, `ALAND` (land area, m²) |
| License | US federal government work, public domain |
| Gives the game | `area_km2` per zone. Polygons (map fills) are not read yet |

**Citation:** U.S. Census Bureau, 2020 TIGER/Line Shapefiles, Block Groups, Texas.

## 6. Agency GTFS static feeds

The feed list is `reference/gtfs_feeds.csv` (`id,agency,url,fallback_url,source`). `fetch.sh` downloads each, tries the fallback (a Mobility Database copy, where one exists) if the agency URL fails, and keeps `routes.txt`, `trips.txt`, `stop_times.txt`, `stops.txt`, `calendar.txt` and `calendar_dates.txt` in `raw/gtfs/<id>/`. A feed that fails only drops that agency's lines. Add an agency by adding a row.

| id | Agency | URL | Fallback |
|---|---|---|---|
| capmetro | Capital Metropolitan Transportation Authority (Austin) | https://data.austintexas.gov/download/r4v4-vz24/application/zip | none |
| dart | Dallas Area Rapid Transit (bus, light rail, streetcar, Silver Line, TRE) | https://www.dart.org/transitdata/latest/google_transit.zip | Mobility Database mdb-152 |
| metro | Metropolitan Transit Authority of Harris County (Houston) | https://metro.resourcespace.com/pages/download.php?ref=4835&ext=zip | Mobility Database mdb-2060 |
| trinity | Trinity Metro (Fort Worth) | https://gtfsdata.ridetm.org/gtfs/fwtatransitdata.zip | Mobility Database mdb-2890 |
| dcta | Denton County Transportation Authority (A-train, buses) | https://gtfs.remix.com/dcta_denton_tx_us.zip | none |
| via | VIA Metropolitan Transit (San Antonio) | https://via.madcollective.com/wp-content/uploads/2025/06/google_transit.zip | none |
| ccrta | Corpus Christi Regional Transportation Authority | https://www.ccrta.org/gtfs/ccrta-gtfs.zip | Mobility Database mdb-315 (service dates end June 2025) |
| epct | El Paso County Transit | https://rapid.nationalrtap.org/GTFSFileManagement/UserUploadFiles/11511/GTFS_EPCT.zip | Mobility Database mdb-3419 |

| | |
|---|---|
| Columns used | `route_short_name`, `route_long_name`, `route_type`; trip `service_id`, `direction_id`; `stop_id`, `stop_sequence` and first `departure_time` per trip; stop name and coordinates; calendar days and exceptions |
| Gives the game | Today's network for every listed agency as existing lines (stops, mode, weekday peak and midday headway, span), and headways on CapMetro routes in `transit.json` |
| Not reachable | From the cloud sandbox (proxy); the Actions runner downloads them |
| Not yet listed | El Paso Sun Metro (city buses and streetcar), Lubbock Citibus, other small-city and rural agencies: no stable static URL found yet |

**Citation:** each agency's GTFS static feed as listed above; Mobility Database (MobilityData) for fallback copies.

## Planned sources (not read yet)

From the overview doc's data table, in the order the pipeline will likely add them:

| Source | For | Milestone |
|---|---|---|
| TIGER/Line block group polygons | Zone polygons for map fills | M1 |
| ACS PUMS | Synthetic population | M2+ |
| NHTS | Non-work trip rates | M2+ |
| TxDOT AADT counts | Road assignment calibration | M3 |
| USGS 3DEP, NLCD | Terrain and land-cover cost multipliers | M3 |
