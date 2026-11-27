//! Guards reference/figures.csv: every figure parses, carries a source URL,
//! and declares how far it was checked.

#[test]
fn reference_figures_are_well_formed() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/reference/figures.csv");
    let mut rdr = csv::Reader::from_path(path).unwrap();
    let mut keys = std::collections::HashSet::new();
    let mut n = 0;
    for rec in rdr.deserialize::<std::collections::HashMap<String, String>>() {
        let r = rec.unwrap();
        let key = &r["key"];
        assert!(keys.insert(key.clone()), "duplicate key {key}");
        let v: f64 = r["value"]
            .parse()
            .unwrap_or_else(|_| panic!("{key}: value not a number"));
        for bound in ["low", "high"] {
            if !r[bound].is_empty() {
                let b: f64 = r[bound]
                    .parse()
                    .unwrap_or_else(|_| panic!("{key}: {bound} not a number"));
                assert!(
                    if bound == "low" { b <= v } else { b >= v },
                    "{key}: {bound} on wrong side of value"
                );
            }
        }
        assert!(
            r["url"].starts_with("https://"),
            "{key}: needs a source URL"
        );
        assert!(
            !r["unit"].is_empty() && !r["source"].is_empty(),
            "{key}: needs unit and source"
        );
        assert!(
            ["verified", "derived", "unverified"].contains(&r["status"].as_str()),
            "{key}: bad status"
        );
        n += 1;
    }
    assert!(n >= 20);
}
