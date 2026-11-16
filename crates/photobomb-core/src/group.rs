//! Groups are derived on demand from stored hashes and pairs, never stored.
//! That is what makes the similarity setting instant: changing it is just a new query.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::{db, hash, now, select, Library, Photo, D_MAX};

/// How far past the threshold a member may sit from its group's centre.
const CHAIN_SLACK: u32 = 2;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct SimilarOpts {
    /// Maximum hash distance, 0 (identical image) to `D_MAX` (very loose).
    pub threshold: u32,
    /// Only group photos with the same shape.
    #[serde(default)]
    pub same_aspect: bool,
    /// Only group photos taken within this many seconds of each other.
    #[serde(default)]
    pub time_window_secs: Option<i64>,
}

impl Default for SimilarOpts {
    fn default() -> Self {
        Self {
            threshold: 8,
            same_aspect: false,
            time_window_secs: None,
        }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct Member {
    #[serde(flatten)]
    pub photo: Photo,
    /// Hash distance to the keeper (similar groups only).
    pub distance: Option<u32>,
    /// Shares its bytes on disk with another member (hard link).
    pub hardlinked: bool,
    /// Byte-identical to the keeper.
    pub exact_copy: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Group {
    pub id: i64,
    pub kind: &'static str,
    pub keeper: i64,
    pub members: Vec<Member>,
    pub reclaimable: i64,
}

#[derive(Serialize, Clone, Debug)]
pub struct GroupPage {
    pub total_groups: usize,
    pub total_photos: usize,
    pub total_reclaimable: i64,
    pub groups: Vec<Group>,
}

#[derive(Serialize, Clone, Debug)]
pub struct PhotoPage {
    pub total: usize,
    pub total_bytes: i64,
    pub photos: Vec<Photo>,
}

pub fn photo_page(photos: Vec<Photo>, page: usize, page_size: usize) -> PhotoPage {
    PhotoPage {
        total: photos.len(),
        total_bytes: photos.iter().map(|p| p.size).sum(),
        photos: photos
            .into_iter()
            .skip(page * page_size)
            .take(page_size)
            .collect(),
    }
}

pub fn page(groups: Vec<Group>, page: usize, page_size: usize) -> GroupPage {
    GroupPage {
        total_groups: groups.len(),
        total_photos: groups.iter().map(|g| g.members.len()).sum(),
        total_reclaimable: groups.iter().map(|g| g.reclaimable).sum(),
        groups: groups
            .into_iter()
            .skip(page * page_size)
            .take(page_size)
            .collect(),
    }
}

/// Every non-keeper in the given groups. Files that are not on disk right now are left
/// alone, and so is any group whose keeper is not available.
pub fn auto_select(groups: &[Group]) -> Vec<&Photo> {
    groups
        .iter()
        .filter(|g| {
            g.members
                .iter()
                .any(|m| m.photo.id == g.keeper && !m.photo.missing)
        })
        .flat_map(|g| {
            g.members
                .iter()
                .map(|m| &m.photo)
                .filter(|p| p.id != g.keeper && !p.missing)
        })
        .collect()
}

fn pair_key(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

fn build_group(
    kind: &'static str,
    mut photos: Vec<Photo>,
    dist: Option<&HashMap<(i64, i64), u32>>,
) -> Group {
    let cmp = if kind == "exact" {
        select::cmp_exact
    } else {
        select::cmp_similar
    };
    // A file that is on disk now always beats one that is not as the suggested keeper.
    photos.sort_by(|a, b| a.missing.cmp(&b.missing).then_with(|| cmp(a, b)));
    let keeper = photos[0].clone();
    let mut inode_count: HashMap<(i64, i64), usize> = HashMap::new();
    for p in &photos {
        *inode_count.entry((p.dev, p.ino)).or_default() += 1;
    }
    // Only files that are on disk now can free any space.
    let reclaimable = if kind == "exact" {
        let live: HashSet<(i64, i64)> = photos
            .iter()
            .filter(|p| !p.missing)
            .map(|p| (p.dev, p.ino))
            .collect();
        keeper.size * (live.len() as i64 - 1).max(0)
    } else {
        photos
            .iter()
            .skip(1)
            .filter(|p| !p.missing)
            .map(|p| p.size)
            .sum()
    };
    let members = photos
        .into_iter()
        .map(|p| Member {
            distance: dist.and_then(|d| {
                if p.id == keeper.id {
                    Some(0)
                } else {
                    d.get(&(p.id.min(keeper.id), p.id.max(keeper.id))).copied()
                }
            }),
            hardlinked: inode_count[&(p.dev, p.ino)] > 1,
            exact_copy: p.id != keeper.id
                && p.content_hash.is_some()
                && p.content_hash == keeper.content_hash,
            photo: p,
        })
        .collect::<Vec<_>>();
    Group {
        id: members.iter().map(|m| m.photo.id).min().unwrap_or(0),
        kind,
        keeper: keeper.id,
        members,
        reclaimable,
    }
}

struct UnionFind(Vec<usize>);
impl UnionFind {
    fn find(&mut self, mut x: usize) -> usize {
        while self.0[x] != x {
            self.0[x] = self.0[self.0[x]];
            x = self.0[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra] = rb;
        }
    }
}

fn components(ids: &[i64], edges: &[(i64, i64)]) -> Vec<Vec<i64>> {
    let index: HashMap<i64, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut uf = UnionFind((0..ids.len()).collect());
    for (a, b) in edges {
        if let (Some(&ia), Some(&ib)) = (index.get(a), index.get(b)) {
            uf.union(ia, ib);
        }
    }
    let mut out: HashMap<usize, Vec<i64>> = HashMap::new();
    for (i, id) in ids.iter().enumerate() {
        out.entry(uf.find(i)).or_default().push(*id);
    }
    out.into_values().filter(|c| c.len() > 1).collect()
}

/// Chain guard: A~B~C~D can link photos that look nothing alike end to end.
/// Members too far from the group's centre are split off and regrouped.
fn split_chains(comp: Vec<i64>, dist: &HashMap<(i64, i64), u32>, t: u32) -> Vec<Vec<i64>> {
    let d = |a: i64, b: i64| -> u32 {
        if a == b {
            0
        } else {
            dist.get(&(a.min(b), a.max(b))).copied().unwrap_or(64)
        }
    };
    let mut out = Vec::new();
    let mut work = vec![comp];
    while let Some(c) = work.pop() {
        if c.len() <= 2 {
            out.push(c);
            continue;
        }
        let medoid = *c
            .iter()
            .min_by_key(|&&m| (c.iter().map(|&x| d(m, x) as u64).sum::<u64>(), m))
            .unwrap();
        let (keep, rest): (Vec<i64>, Vec<i64>) =
            c.iter().partition(|&&m| d(medoid, m) <= t + CHAIN_SLACK);
        if rest.is_empty() {
            out.push(keep);
            continue;
        }
        if keep.len() > 1 {
            out.push(keep);
        }
        let rest_set: HashSet<i64> = rest.iter().copied().collect();
        let edges: Vec<(i64, i64)> = dist
            .iter()
            .filter(|((a, b), dd)| **dd <= t && rest_set.contains(a) && rest_set.contains(b))
            .map(|((a, b), _)| (*a, *b))
            .collect();
        work.extend(components(&rest, &edges));
    }
    out
}

impl Library {
    fn ignored(&self) -> Result<HashSet<(String, String)>> {
        let conn = self.db();
        let mut stmt = conn.prepare("SELECT hash_a, hash_b FROM ignored_pair")?;
        let set = stmt
            .query_map([], |r| {
                Ok((
                    hex::encode(r.get::<_, Vec<u8>>(0)?),
                    hex::encode(r.get::<_, Vec<u8>>(1)?),
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(set)
    }

    /// Groups of byte-identical files, biggest saving first.
    pub fn exact_groups(&self) -> Result<Vec<Group>> {
        let ignored = self.ignored()?;
        let photos = db::photos_where(
            &self.db(),
            "state = 'present' AND not_dup = 0 AND content_hash IS NOT NULL ORDER BY content_hash",
        )?;
        let mut by_hash: HashMap<String, Vec<Photo>> = HashMap::new();
        for p in photos {
            by_hash
                .entry(p.content_hash.clone().unwrap())
                .or_default()
                .push(p);
        }
        let mut groups: Vec<Group> = by_hash
            .into_iter()
            .filter(|(h, v)| v.len() > 1 && !ignored.contains(&(h.clone(), h.clone())))
            .map(|(_, v)| build_group("exact", v, None))
            .collect();
        groups.sort_by(|a, b| b.reclaimable.cmp(&a.reclaimable).then(a.id.cmp(&b.id)));
        Ok(groups)
    }

    /// Groups of visually similar photos at the given similarity level.
    pub fn similar_groups(&self, opts: &SimilarOpts) -> Result<Vec<Group>> {
        let t = opts.threshold.min(D_MAX);
        let ignored = self.ignored()?;
        let (photos, pairs) = {
            let conn = self.db();
            let photos: HashMap<i64, Photo> =
                db::photos_where(&conn, "state = 'present' AND not_dup = 0 AND decoded = 1")?
                    .into_iter()
                    .map(|p| (p.id, p))
                    .collect();
            let mut stmt =
                conn.prepare("SELECT a_id, b_id, distance FROM similar_pair WHERE distance <= ?1")?;
            // Pairs slightly past the threshold are needed by the chain guard.
            let pairs: Vec<(i64, i64, u32)> = stmt
                .query_map([t + CHAIN_SLACK], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            (photos, pairs)
        };

        let allowed = |a: &Photo, b: &Photo| -> bool {
            if opts.same_aspect {
                let ratio =
                    |p: &Photo| p.width.unwrap_or(1) as f64 / p.height.unwrap_or(1).max(1) as f64;
                let (ra, rb) = (ratio(a), ratio(b));
                if (ra - rb).abs() / ra.max(rb) > 0.03 {
                    return false;
                }
            }
            if let Some(win) = opts.time_window_secs {
                match (a.taken_at, b.taken_at) {
                    (Some(x), Some(y)) if (x - y).abs() <= win => {}
                    _ => return false,
                }
            }
            if let (Some(ha), Some(hb)) = (&a.content_hash, &b.content_hash) {
                if ignored.contains(&pair_key(ha, hb)) {
                    return false;
                }
            }
            true
        };

        let mut dist: HashMap<(i64, i64), u32> = HashMap::new();
        let mut edges: Vec<(i64, i64)> = Vec::new();
        for (a, b, d) in pairs {
            let (Some(pa), Some(pb)) = (photos.get(&a), photos.get(&b)) else {
                continue;
            };
            if !allowed(pa, pb) {
                continue;
            }
            dist.insert((a, b), d);
            if d <= t {
                edges.push((a, b));
            }
        }
        let mut ids: Vec<i64> = edges.iter().flat_map(|(a, b)| [*a, *b]).collect();
        ids.sort_unstable();
        ids.dedup();

        let mut groups = Vec::new();
        for comp in components(&ids, &edges) {
            for part in split_chains(comp, &dist, t) {
                let members: Vec<Photo> = part
                    .iter()
                    .filter_map(|id| photos.get(id).cloned())
                    .collect();
                if members.len() < 2 {
                    continue;
                }
                // Pure byte-identical sets already show up under exact duplicates.
                let first = &members[0].content_hash;
                if first.is_some() && members.iter().all(|m| &m.content_hash == first) {
                    continue;
                }
                groups.push(build_group("similar", members, Some(&dist)));
            }
        }
        groups.sort_by(|a, b| b.reclaimable.cmp(&a.reclaimable).then(a.id.cmp(&b.id)));
        Ok(groups)
    }

    /// Returns the photo's content hash, computing and storing it if needed.
    pub(crate) fn ensure_content_hash(&self, id: i64) -> Result<Vec<u8>> {
        let (path, existing): (String, Option<Vec<u8>>) = self.db().query_row(
            "SELECT path, content_hash FROM photo WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if let Some(h) = existing {
            return Ok(h);
        }
        let h = hash::full_hash(Path::new(&path))
            .with_context(|| format!("cannot read {path}"))?
            .to_vec();
        self.db().execute(
            "UPDATE photo SET content_hash = ?2 WHERE id = ?1",
            params![id, h],
        )?;
        Ok(h)
    }

    /// Marks these photos as "not duplicates of each other". Survives moves and rescans.
    pub fn ignore_group(&self, ids: &[i64]) -> Result<()> {
        let mut hashes = Vec::new();
        for id in ids {
            hashes.push(self.ensure_content_hash(*id)?);
        }
        let conn = self.db();
        for i in 0..hashes.len() {
            for j in i..hashes.len() {
                let (a, b) = if hashes[i] <= hashes[j] {
                    (&hashes[i], &hashes[j])
                } else {
                    (&hashes[j], &hashes[i])
                };
                if i != j || hashes.iter().filter(|h| *h == a).count() > 1 {
                    conn.execute(
                        "INSERT OR IGNORE INTO ignored_pair (hash_a, hash_b, created_at) VALUES (?1,?2,?3)",
                        params![a, b, now()],
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Marks one photo as "not a duplicate" of the other members of its group; the rest
    /// of the group stays together.
    ///
    /// When the photo differs in content from every other member, the decision is stored
    /// as content-hash pairs, so it survives moves and the photo can still match files
    /// added later. A byte-identical copy cannot be told apart by content, so there the
    /// file itself is flagged and left out of every group.
    pub fn ignore_photo(&self, id: i64, others: &[i64]) -> Result<()> {
        let others: Vec<i64> = others.iter().copied().filter(|o| *o != id).collect();
        let mine = self.ensure_content_hash(id).ok();
        let theirs: Vec<Option<Vec<u8>>> = others
            .iter()
            .map(|o| self.ensure_content_hash(*o).ok())
            .collect();
        let by_content = match &mine {
            Some(m) => {
                !others.is_empty() && theirs.iter().all(|t| t.as_ref().is_some_and(|t| t != m))
            }
            None => false,
        };
        let conn = self.db();
        if by_content {
            let m = mine.unwrap();
            for t in theirs.into_iter().flatten() {
                let (a, b) = if m <= t { (&m, &t) } else { (&t, &m) };
                conn.execute(
                    "INSERT OR IGNORE INTO ignored_pair (hash_a, hash_b, created_at) VALUES (?1,?2,?3)",
                    params![a, b, now()],
                )?;
            }
        } else {
            let n = conn.execute("UPDATE photo SET not_dup = 1 WHERE id = ?1", [id])?;
            anyhow::ensure!(n == 1, "unknown photo");
        }
        Ok(())
    }

    /// Puts a flagged photo back into duplicate matching.
    pub fn unignore_photo(&self, id: i64) -> Result<()> {
        self.db()
            .execute("UPDATE photo SET not_dup = 0 WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Photos that are in no exact group and no similar group at this similarity level.
    pub fn other_photos(&self, opts: &SimilarOpts) -> Result<Vec<Photo>> {
        let mut grouped: HashSet<i64> = HashSet::new();
        for g in self
            .exact_groups()?
            .iter()
            .chain(self.similar_groups(opts)?.iter())
        {
            grouped.extend(g.members.iter().map(|m| m.photo.id));
        }
        let mut photos = db::photos_where(&self.db(), "state = 'present' ORDER BY path")?;
        photos.retain(|p| !grouped.contains(&p.id));
        Ok(photos)
    }

    /// Saved "not duplicates" decisions: ignored pairs plus individually flagged photos.
    pub fn ignored_count(&self) -> Result<i64> {
        Ok(self.db().query_row(
            "SELECT (SELECT COUNT(*) FROM ignored_pair) + (SELECT COUNT(*) FROM photo WHERE not_dup = 1)",
            [],
            |r| r.get(0),
        )?)
    }

    pub fn clear_ignored(&self) -> Result<()> {
        let conn = self.db();
        conn.execute("DELETE FROM ignored_pair", [])?;
        conn.execute("UPDATE photo SET not_dup = 0 WHERE not_dup = 1", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_is_split() {
        // 1-2-3-4 chained at distance 4 each, ends far apart.
        let mut dist = HashMap::new();
        dist.insert((1, 2), 4);
        dist.insert((2, 3), 4);
        dist.insert((3, 4), 4);
        dist.insert((1, 3), 8);
        dist.insert((2, 4), 8);
        dist.insert((1, 4), 12);
        let parts = split_chains(vec![1, 2, 3, 4], &dist, 4);
        assert!(parts.iter().all(|p| p.len() < 4), "{parts:?}");
        // A tight cluster stays whole.
        let mut tight = HashMap::new();
        for (a, b) in [(1, 2), (1, 3), (2, 3)] {
            tight.insert((a, b), 3);
        }
        assert_eq!(split_chains(vec![1, 2, 3], &tight, 4).len(), 1);
    }
}
