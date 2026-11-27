# Reference figures

Decision D18. The overview design doc used cost, fare, ridership and zone-count figures "from memory, to be replaced with sourced numbers." This page and [`reference/figures.csv`](../reference/figures.csv) replace them. The CSV is the machine-readable source; this page explains what changed and what is still open.

Collected 2026-11-27 from the pages linked in the CSV.

## How to read the CSV

| Column | Meaning |
|---|---|
| `key` | Stable identifier, safe to reference from code (sim-core defaults, balance tables) |
| `value` | Best single figure |
| `low`, `high` | Range where the sources give one; empty otherwise |
| `unit`, `dollar_year` | Units, and the year of the dollars. Inflate before mixing years. sim-core's schema uses 2024 USD |
| `scope` | What the number describes (Texas, US, one project, one agency) |
| `status` | `verified`: number read directly from the source page. `derived`: computed from verified numbers (the note shows the arithmetic). `unverified`: read from an extracted summary of a long page, or a fetch failed, so recheck before relying on it |
| `source`, `url`, `note` | Citation and caveats |

`tests/reference.rs` checks that every row parses, has a URL and a status, and that `low ≤ value ≤ high`.

## What changed against the design doc

| Design doc said | Sourced figure | Verdict |
|---|---|---|
| Texas ~6,900 tracts, ~18,600 block groups, ~670,000 blocks | 6,896 / 18,638 / 668,757 (2020) | Doc was right. Counts are `unverified` until a `--area 48` build confirms them |
| ~31 million people | 29,145,505 in the 2020 census | The doc's figure is a recent estimate; the pipeline uses the 2020 count |
| 2-lane road $3–5M/mi rural, $8–15M urban | $5.5M/mi rural 2-lane, $17.0M/mi urban 4-lane divided (FDOT models, excluding right-of-way and structures) | Doc low. Rural top of range; urban above it before right-of-way |
| Freeway lane added $5–10M rural, $15–40M urban per mile | Urban 4→6 widening $16.0M per centerline mile, about $8M per lane-mile (FDOT) | Doc's $15–40M urban range is per mile; FDOT's $16M sits at its bottom because it excludes interchanges and right-of-way, which dominate Texas urban freeway costs. Keep the doc's range |
| Light rail $100–250M/mi urban | US average rail transit $172M/mi; mostly at-grade $117M/mi (2019 USD, Eno). Austin Phase 1: $840M/mi YOE ($729M before finance charges) | Doc range fits US averages. Austin's real project is 3–4x higher; worth a scenario that uses it |
| Subway $500M–1.5B/mi | Mostly underground US projects $570M/mi (2019 USD, ex-NYC); NYC average much higher | Doc range is right |
| High-speed rail $150M+/mi urban | Texas Central: >$40B over 240 mi ≈ $167M/mi | Consistent |
| Bus $0.6–1M each | Diesel 40-ft $716k planning price ($500k median paid); battery-electric $1.41M ($1.1M median) | Doc fine for diesel; electric sits above its top |
| Rail car $3–5M | LRV $5.0M (SacRT) to $7.1M (St. Louis, incl. support) | Doc low |
| Bus operating $150–200 per revenue hour | $180–185 in Austin, Dallas, Houston (NTD 2023) | Doc right |
| Light rail operating $300–450 per revenue hour | $424 (DART) and $448 (METRO) | Doc right, top of range |
| — | Commuter/hybrid rail $716 (DART) to $1,227 (CapMetro Red Line) per revenue hour | New. Low-frequency rail is expensive per hour |
| Fares $1.25–2.50 | CapMetro $1.25 local ($1.50 from Jan 2027), $3.50 rail; METRO $1.25; VIA $1.30 | Doc right for local; rail/express fares are higher |
| — | Farebox recovery 4.8% (METRO) to 5.8% (DART), 2023 | New. Fares cover about 5% of operating cost in big Texas systems, which matters for the budget model |
| Tolls $0.20–0.50/mi | Published as per-plaza tolls (183A Park Street $2.00 with TxTag, 2026); no per-mile rate found | Still unverified |
| ~3.5 trips per person per day (NHTS) | 2.28 (2022 NHTS), 3.37 (2017) | Doc used the 2017 value. 2022 is post-pandemic and changed survey method; use 3.37 as the steady-state default and 2.28 as a low case |
| Value of time ~$18/h, wait and walk ~2x | 2x multiplier confirmed (USDOT BCA 2025 guidance, from a summary); hourly dollar value not yet extracted | Still open |
| — | CapMetro MetroRail 477,322 annual trips (2023); Austin light rail forecast 18,700 weekday trips at opening | New. These are the M1 calibration targets |

## Still open

1. **Value of travel time in dollars.** USDOT BCA Guidance Appendix A Table A-2 (2023 USD). The fetch tools could not read the appendix pages.
2. **Toll per mile.** Needs segment lengths for CTRMA/TxDOT plazas, or TxDOT's published per-mile schedule for SH 130 / SH 45.
3. **DART local fare.** DART's fare pages did not list the local single-ride price in readable text.
4. **Texas-specific road costs.** FDOT's models stand in for Texas. TxDOT bid tabulations would give Texas unit costs.
5. **Dollar-year normalization.** Figures span 2019–2027 dollars. A small helper to inflate to 2024 USD (CPI-U) is needed before sim-core consumes them.

## Updating

Add or change a row in the CSV, run `cargo test --test reference`, and update the table above when the change moves a design-doc number. Never delete a source: if a figure is superseded, keep the old row under a new key with a note, so balance changes stay traceable.
