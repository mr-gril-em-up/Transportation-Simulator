"""Downloads TxDOT's AADT Annuals station layer to CSV (F38, DATA-10).

Usage: python3 scripts/txdot_aadt.py OUT.csv

Source: TxDOT Transportation Planning and Programming, "TxDOT AADT Annuals
(Public View)" ArcGIS feature service, layer 0 (one point per traffic count
station). Only stations with a positive AADT are kept.
"""
import csv
import json
import sys
import time
import urllib.parse
import urllib.request

URL = ("https://services.arcgis.com/KTcxiTD9dsQw4r7Z/arcgis/rest/services/"
       "TxDOT_AADT_Annuals_(Public_View)/FeatureServer/0/query")
FIELDS = ["TRFC_STATN_ID", "AADT_RPT_YEAR", "AADT_RPT_QTY", "ON_ROAD",
          "CNTY_NM", "ACTIVE", "LONGITUDE", "LATITUDE"]


def page(offset):
    q = urllib.parse.urlencode({
        "where": "AADT_RPT_QTY > 0", "outFields": ",".join(FIELDS),
        "returnGeometry": "false", "orderByFields": "OBJECTID_1",
        "resultOffset": offset, "resultRecordCount": 2000, "f": "json"})
    for attempt in range(5):
        try:
            with urllib.request.urlopen(URL + "?" + q, timeout=120) as r:
                d = json.load(r)
            if "error" in d:
                raise RuntimeError(d["error"])
            return d
        except Exception as e:  # network or service error: retry
            print(f"retry {attempt + 1}: {e}", file=sys.stderr)
            time.sleep(10 * (attempt + 1))
    raise SystemExit("TxDOT AADT download failed")


def main(out):
    rows, offset = [], 0
    while True:
        d = page(offset)
        feats = d.get("features", [])
        rows += [f["attributes"] for f in feats]
        if not feats or not d.get("exceededTransferLimit"):
            break
        offset += len(feats)
    with open(out, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=FIELDS)
        w.writeheader()
        for r in rows:
            w.writerow({k: r.get(k) for k in FIELDS})
    print(f"     {len(rows)} TxDOT count stations")
    roads = sorted({str(r.get("ON_ROAD")) for r in rows})
    print("     ON_ROAD sample:", roads[:: max(1, len(roads) // 60)][:60])


if __name__ == "__main__":
    main(sys.argv[1])
