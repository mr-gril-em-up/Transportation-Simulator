//! The zone pairs the model keeps (F34). Every per-pair quantity (skims,
//! demand, flows) is a flat vector indexed by pair, in compressed rows: origin
//! `i` owns pairs `row(i)`, and pair `k` goes to zone `dest[k]`. With no
//! cutoff every pair is kept, which is the dense Z x Z model; with
//! `Params::strategic_max_min` only destinations within that free-flow car
//! time are, so storage grows with zones x nearby zones instead of zones².

use std::ops::Range;

/// Compressed-row set of kept origin-destination pairs. Symmetric: if `(i, j)`
/// is kept so is `(j, i)`, and `rev[k]` is the reverse pair's index. With a
/// cutoff a pair is kept only if both directions are within it, so every kept
/// pair is drivable both ways.
#[derive(Debug, Clone, Default)]
pub struct Pairs {
    start: Vec<usize>,
    /// Destination zone of each pair; ascending within a row.
    pub dest: Vec<u32>,
    /// Index of the reverse pair `(j, i)` for each pair `(i, j)`.
    pub rev: Vec<usize>,
}

impl Pairs {
    /// Every pair of `z` zones, intrazonal included.
    pub fn dense(z: usize) -> Pairs {
        Pairs::from_rows((0..z).map(|_| (0..z as u32).collect()).collect())
    }

    /// From per-origin destination lists: pair `(i, j)` is kept when `j` is
    /// in row `i` **and** `i` is in row `j` (each zone reaches the other), so
    /// the set is symmetric. The intrazonal pair is always kept.
    pub fn from_rows(mut rows: Vec<Vec<u32>>) -> Pairs {
        let z = rows.len();
        for (i, r) in rows.iter_mut().enumerate() {
            r.push(i as u32);
            r.sort_unstable();
            r.dedup();
        }
        let has = |rows: &Vec<Vec<u32>>, j: usize, i: u32| rows[j].binary_search(&i).is_ok();
        let kept: Vec<Vec<u32>> = (0..z)
            .map(|i| {
                rows[i]
                    .iter()
                    .copied()
                    .filter(|&j| has(&rows, j as usize, i as u32))
                    .collect()
            })
            .collect();
        let mut start = Vec::with_capacity(z + 1);
        let mut dest = Vec::new();
        start.push(0);
        for r in &kept {
            dest.extend_from_slice(r);
            start.push(dest.len());
        }
        let mut p = Pairs {
            start,
            dest,
            rev: Vec::new(),
        };
        p.rev = (0..z)
            .flat_map(|i| p.row(i).map(move |k| (i, k)))
            .map(|(i, k)| p.find(p.dest[k] as usize, i).expect("symmetric"))
            .collect();
        p
    }

    /// Number of kept pairs.
    pub fn len(&self) -> usize {
        self.dest.len()
    }

    /// Pair indices of origin `i`.
    pub fn row(&self, i: usize) -> Range<usize> {
        self.start[i]..self.start[i + 1]
    }

    /// Destinations of origin `i`.
    pub fn dests(&self, i: usize) -> &[u32] {
        &self.dest[self.row(i)]
    }

    /// Index of pair `(i, j)`, if kept.
    pub fn find(&self, i: usize, j: usize) -> Option<usize> {
        let r = self.row(i);
        self.dest[r.clone()]
            .binary_search(&(j as u32))
            .ok()
            .map(|q| r.start + q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_symmetric_with_reverse_index() {
        // 0 and 2 reach each other; 1 reaches 0 but 0 does not reach 1.
        let p = Pairs::from_rows(vec![vec![2], vec![0], vec![0]]);
        assert_eq!(p.dests(0), &[0, 2]);
        assert_eq!(p.dests(1), &[1]);
        assert_eq!(p.dests(2), &[0, 2]);
        for i in 0..3 {
            for k in p.row(i) {
                let j = p.dest[k] as usize;
                assert_eq!(p.dest[p.rev[k]] as usize, i);
                assert!(p.row(j).contains(&p.rev[k]));
            }
        }
        assert_eq!(p.find(0, 1), None);
        assert_eq!(Pairs::dense(4).len(), 16);
        assert_eq!(Pairs::dense(4).find(3, 1), Some(13));
    }
}
