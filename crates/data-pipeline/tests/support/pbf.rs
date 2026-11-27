//! Minimal OSM PBF writer for test fixtures (uncompressed blobs, plain nodes).
//! Lets tests describe a tiny map in code instead of committing binary files.
//! Format: https://wiki.openstreetmap.org/wiki/PBF_Format

fn varint(buf: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            buf.push(b);
            return;
        }
        buf.push(b | 0x80);
    }
}
fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}
fn key(buf: &mut Vec<u8>, field: u32, wire: u32) {
    varint(buf, ((field << 3) | wire) as u64);
}
fn bytes(buf: &mut Vec<u8>, field: u32, data: &[u8]) {
    key(buf, field, 2);
    varint(buf, data.len() as u64);
    buf.extend_from_slice(data);
}
fn uint(buf: &mut Vec<u8>, field: u32, v: u64) {
    key(buf, field, 0);
    varint(buf, v);
}
fn packed(buf: &mut Vec<u8>, field: u32, vals: impl IntoIterator<Item = u64>) {
    let mut p = Vec::new();
    for v in vals {
        varint(&mut p, v);
    }
    bytes(buf, field, &p);
}

pub enum Member {
    Node(i64, &'static str),
    Way(i64, &'static str),
}

type Tags = Vec<(&'static str, &'static str)>;

#[derive(Default)]
pub struct Map {
    pub nodes: Vec<(i64, f64, f64, Tags)>,
    pub ways: Vec<(i64, Vec<i64>, Tags)>,
    pub relations: Vec<(i64, Vec<Member>, Tags)>,
}

impl Map {
    pub fn node(&mut self, id: i64, lon: f64, lat: f64, tags: &[(&'static str, &'static str)]) {
        self.nodes.push((id, lon, lat, tags.to_vec()));
    }
    pub fn way(&mut self, id: i64, refs: &[i64], tags: &[(&'static str, &'static str)]) {
        self.ways.push((id, refs.to_vec(), tags.to_vec()));
    }
    pub fn relation(
        &mut self,
        id: i64,
        members: Vec<Member>,
        tags: &[(&'static str, &'static str)],
    ) {
        self.relations.push((id, members, tags.to_vec()));
    }

    pub fn to_pbf(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut header = Vec::new();
        bytes(&mut header, 4, b"OsmSchema-V0.6");
        blob(&mut out, "OSMHeader", &header);

        let mut strings: Vec<&str> = vec![""];
        let mut sid = |s: &'static str| -> u64 {
            if let Some(i) = strings.iter().position(|x| *x == s) {
                return i as u64;
            }
            strings.push(s);
            (strings.len() - 1) as u64
        };
        let mut group = Vec::new();
        for (id, lon, lat, tags) in &self.nodes {
            let mut n = Vec::new();
            uint(&mut n, 1, zigzag(*id));
            packed(&mut n, 2, tags.iter().map(|t| sid(t.0)));
            packed(&mut n, 3, tags.iter().map(|t| sid(t.1)));
            // granularity 100 nanodegrees
            uint(&mut n, 8, zigzag((lat * 1e7).round() as i64));
            uint(&mut n, 9, zigzag((lon * 1e7).round() as i64));
            bytes(&mut group, 1, &n);
        }
        for (id, refs, tags) in &self.ways {
            let mut w = Vec::new();
            uint(&mut w, 1, *id as u64);
            packed(&mut w, 2, tags.iter().map(|t| sid(t.0)));
            packed(&mut w, 3, tags.iter().map(|t| sid(t.1)));
            let mut prev = 0;
            packed(
                &mut w,
                8,
                refs.iter().map(|r| {
                    let d = r - prev;
                    prev = *r;
                    zigzag(d)
                }),
            );
            bytes(&mut group, 3, &w);
        }
        for (id, members, tags) in &self.relations {
            let mut r = Vec::new();
            uint(&mut r, 1, *id as u64);
            packed(&mut r, 2, tags.iter().map(|t| sid(t.0)));
            packed(&mut r, 3, tags.iter().map(|t| sid(t.1)));
            let parts: Vec<(i64, &'static str, u64)> = members
                .iter()
                .map(|m| match m {
                    Member::Node(id, role) => (*id, *role, 0),
                    Member::Way(id, role) => (*id, *role, 1),
                })
                .collect();
            packed(&mut r, 8, parts.iter().map(|p| sid(p.1)));
            let mut prev = 0;
            packed(
                &mut r,
                9,
                parts.iter().map(|p| {
                    let d = p.0 - prev;
                    prev = p.0;
                    zigzag(d)
                }),
            );
            packed(&mut r, 10, parts.iter().map(|p| p.2));
            bytes(&mut group, 4, &r);
        }
        let mut st = Vec::new();
        for s in &strings {
            bytes(&mut st, 1, s.as_bytes());
        }
        let mut block = Vec::new();
        bytes(&mut block, 1, &st);
        bytes(&mut block, 2, &group);
        blob(&mut out, "OSMData", &block);
        out
    }
}

fn blob(out: &mut Vec<u8>, kind: &str, data: &[u8]) {
    let mut b = Vec::new();
    bytes(&mut b, 1, data); // raw
    let mut h = Vec::new();
    bytes(&mut h, 1, kind.as_bytes());
    uint(&mut h, 3, b.len() as u64);
    out.extend_from_slice(&(h.len() as u32).to_be_bytes());
    out.extend_from_slice(&h);
    out.extend_from_slice(&b);
}
