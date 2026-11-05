//! Offline driving directions (ADR 0013).
//!
//! Build time (CI): OSM extract → car-routable ways → graph whose nodes are road
//! junctions, with each edge keeping the road geometry between its junctions → keep
//! the largest connected component → contraction hierarchies (`fast_paths`) → files
//! under `maps/routing/` in a signed map package.
//!
//! Run time: load the files, snap points to the nearest junction, query, return the
//! route geometry, distance and estimated time. v0 ignores turn restrictions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use thiserror::Error;

pub const ROUTING_DIR: &str = "routing";
const GRAPH: &str = "graph.bin.gz";
const NODES: &str = "nodes.bin.gz";
const EDGES: &str = "edges.bin.gz";
const GEOM: &str = "geom.bin.gz";

/// The contraction-hierarchy graph stored with 32-bit fields (half the size of the default).
#[derive(Serialize, Deserialize)]
struct Graph32 {
    #[serde(serialize_with = "fast_paths::serialize_32", deserialize_with = "fast_paths::deserialize_32")]
    fg: fast_paths::FastGraph,
}

fn write_gz<T: Serialize>(path: &Path, value: &T) -> Result<(), RoutingError> {
    let f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut gz = flate2::write::GzEncoder::new(f, flate2::Compression::new(6));
    bincode::serialize_into(&mut gz, value)?;
    gz.finish()?;
    Ok(())
}

fn read_gz<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, RoutingError> {
    let f = std::io::BufReader::new(std::fs::File::open(path)?);
    Ok(bincode::deserialize_from(flate2::read::GzDecoder::new(f))?)
}
const META: &str = "meta.json";
/// Snapping grid cell, in degrees (about 1.1 km of latitude).
const CELL: f64 = 0.01;

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("encoding: {0}")]
    Encoding(#[from] bincode::Error),
    #[error("osm: {0}")]
    Osm(String),
    #[error("meta: {0}")]
    Meta(#[from] serde_json::Error),
}

/// One way as the builder needs it.
#[derive(Debug, Clone)]
pub struct RoadWay {
    pub nodes: Vec<i64>,
    pub speed_kmh: f32,
    /// 1 = forward only, -1 = backward only, 0 = both directions.
    pub oneway: i8,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EdgeRec {
    pub from: u32,
    pub to: u32,
    pub geom_off: u32,
    pub geom_len: u32,
    pub reversed: bool,
    pub dist_m: f32,
    pub time_s: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub format: u32,
    pub profile: String,
    pub nodes: usize,
    pub edges: usize,
    pub ways: usize,
    pub dropped_nodes: usize,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Route {
    pub distance_m: f64,
    pub duration_s: f64,
    /// [lon, lat] pairs, ready for GeoJSON.
    pub coordinates: Vec<[f64; 2]>,
    /// How far each endpoint had to move to reach the road network.
    pub snap_from_m: f64,
    pub snap_to_m: f64,
}

/// Great-circle distance between two [lat, lon] points, in metres.
pub fn haversine_m(a: [f64; 2], b: [f64; 2]) -> f64 {
    let r = 6_371_000.0f64;
    let (la1, la2) = (a[0].to_radians(), b[0].to_radians());
    let dla = la2 - la1;
    let dlo = (b[1] - a[1]).to_radians();
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * r * h.sqrt().asin()
}

// ---------- OSM tag rules (car profile) ----------

/// Typical car speed on this way in km/h, or None if cars can't use it.
pub fn car_speed(tags: &HashMap<String, String>) -> Option<f32> {
    let hw = tags.get("highway")?.as_str();
    let base: f32 = match hw {
        "motorway" => 105.0,
        "trunk" => 90.0,
        "primary" => 80.0,
        "secondary" => 70.0,
        "tertiary" => 60.0,
        "unclassified" => 50.0,
        "residential" => 40.0,
        "living_street" => 15.0,
        "service" => 20.0,
        "motorway_link" => 70.0,
        "trunk_link" => 60.0,
        "primary_link" => 55.0,
        "secondary_link" => 50.0,
        "tertiary_link" => 45.0,
        _ => return None,
    };
    let no = |k: &str| matches!(tags.get(k).map(String::as_str), Some("no" | "private"));
    if no("access") || no("motor_vehicle") || no("motorcar") {
        return None;
    }
    if hw == "service"
        && matches!(tags.get("service").map(String::as_str), Some("driveway" | "parking_aisle" | "drive-through"))
    {
        return None;
    }
    if tags.get("area").map(String::as_str) == Some("yes") {
        return None;
    }
    let posted = tags.get("maxspeed").and_then(|v| {
        let num: f32 = v.split_whitespace().next()?.parse().ok()?;
        Some(if v.contains("mph") { num * 1.609 } else { num })
    });
    // Typical speed sits below the posted limit.
    Some(posted.map(|p| (p * 0.9).min(base * 1.3)).unwrap_or(base))
}

pub fn oneway(tags: &HashMap<String, String>) -> i8 {
    match tags.get("oneway").map(String::as_str) {
        Some("yes" | "1" | "true") => 1,
        Some("-1" | "reverse") => -1,
        Some("no") => 0,
        _ => {
            let hw = tags.get("highway").map(String::as_str).unwrap_or("");
            if hw == "motorway" || tags.get("junction").map(String::as_str) == Some("roundabout") {
                1
            } else {
                0
            }
        }
    }
}

// ---------- build ----------

struct UnionFind(Vec<u32>);

impl UnionFind {
    fn new(n: usize) -> Self {
        Self((0..n as u32).collect())
    }
    fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x {
            self.0[x as usize] = self.0[self.0[x as usize] as usize];
            x = self.0[x as usize];
        }
        x
    }
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.0[ra as usize] = rb;
        }
    }
}

fn junction_id(n: i64, c: [f64; 2], jid: &mut HashMap<i64, u32>, jcoord: &mut Vec<[f64; 2]>) -> u32 {
    *jid.entry(n).or_insert_with(|| {
        jcoord.push(c);
        (jcoord.len() - 1) as u32
    })
}

/// Build routing files from ways and node coordinates ([lat, lon]).
pub fn build(ways: &[RoadWay], coords: &HashMap<i64, [f64; 2]>, out: &Path) -> Result<Meta, RoutingError> {
    // Junctions: way endpoints and nodes used more than once across ways.
    let mut uses: HashMap<i64, u32> = HashMap::new();
    for w in ways {
        for n in &w.nodes {
            *uses.entry(*n).or_default() += 1;
        }
    }
    let mut jid: HashMap<i64, u32> = HashMap::new();
    let mut jcoord: Vec<[f64; 2]> = Vec::new();
    let mut geom: Vec<[f32; 2]> = Vec::new();
    let mut raw: Vec<EdgeRec> = Vec::new();

    for w in ways {
        let nodes: Vec<i64> = w.nodes.iter().copied().filter(|n| coords.contains_key(n)).collect();
        if nodes.len() < 2 {
            continue;
        }
        let mut start = 0usize;
        for i in 1..nodes.len() {
            let is_junction = i == nodes.len() - 1 || uses.get(&nodes[i]).copied().unwrap_or(0) > 1;
            if !is_junction {
                continue;
            }
            let seg = &nodes[start..=i];
            let a = junction_id(seg[0], coords[&seg[0]], &mut jid, &mut jcoord);
            let b = junction_id(seg[seg.len() - 1], coords[&seg[seg.len() - 1]], &mut jid, &mut jcoord);
            if a != b {
                let off = geom.len() as u32;
                let mut dist = 0.0;
                for k in 0..seg.len() {
                    let c = coords[&seg[k]];
                    geom.push([c[0] as f32, c[1] as f32]);
                    if k > 0 {
                        dist += haversine_m(coords[&seg[k - 1]], c);
                    }
                }
                let time = dist / (w.speed_kmh as f64 / 3.6);
                let rec = |from, to, reversed| EdgeRec {
                    from,
                    to,
                    geom_off: off,
                    geom_len: seg.len() as u32,
                    reversed,
                    dist_m: dist as f32,
                    time_s: time as f32,
                };
                if w.oneway >= 0 {
                    raw.push(rec(a, b, false));
                }
                if w.oneway <= 0 {
                    raw.push(rec(b, a, true));
                }
            }
            start = i;
        }
    }

    // Keep the largest connected component so every snapped point can reach every other.
    let n_all = jcoord.len();
    let mut uf = UnionFind::new(n_all);
    for e in &raw {
        uf.union(e.from, e.to);
    }
    let mut size: HashMap<u32, usize> = HashMap::new();
    for i in 0..n_all as u32 {
        *size.entry(uf.find(i)).or_default() += 1;
    }
    let main = size.iter().max_by_key(|(_, s)| **s).map(|(r, _)| *r).unwrap_or(0);
    let mut remap = vec![u32::MAX; n_all];
    let mut nodes: Vec<[f32; 2]> = Vec::new();
    for i in 0..n_all as u32 {
        if uf.find(i) == main {
            remap[i as usize] = nodes.len() as u32;
            let c = jcoord[i as usize];
            nodes.push([c[0] as f32, c[1] as f32]);
        }
    }
    let mut edges: Vec<EdgeRec> = raw
        .into_iter()
        .filter(|e| remap[e.from as usize] != u32::MAX && remap[e.to as usize] != u32::MAX)
        .map(|mut e| {
            e.from = remap[e.from as usize];
            e.to = remap[e.to as usize];
            e
        })
        .collect();
    // One edge per (from, to): the fastest.
    edges.sort_by(|a, b| (a.from, a.to).cmp(&(b.from, b.to)).then(a.time_s.total_cmp(&b.time_s)));
    edges.dedup_by(|b, a| a.from == b.from && a.to == b.to);

    let mut input = fast_paths::InputGraph::new();
    for e in &edges {
        input.add_edge(e.from as usize, e.to as usize, ((e.time_s * 10.0).round() as usize).max(1));
    }
    input.freeze();
    let fg = fast_paths::prepare(&input);

    std::fs::create_dir_all(out)?;
    write_gz(&out.join(GRAPH), &Graph32 { fg })?;
    write_gz(&out.join(NODES), &nodes)?;
    write_gz(&out.join(EDGES), &edges)?;
    write_gz(&out.join(GEOM), &geom)?;
    let meta = Meta {
        format: 2,
        profile: "car".into(),
        nodes: nodes.len(),
        edges: edges.len(),
        ways: ways.len(),
        dropped_nodes: n_all - nodes.len(),
        notes: "Turn restrictions and time-of-day rules are not modelled.".into(),
    };
    std::fs::write(out.join(META), serde_json::to_vec_pretty(&meta)?)?;
    Ok(meta)
}

/// Read car-routable ways and their node coordinates from an .osm.pbf, then `build`.
pub fn build_from_pbf(pbf: &Path, out: &Path) -> Result<Meta, RoutingError> {
    use osmpbf::{Element, ElementReader};
    let osm = |e: osmpbf::Error| RoutingError::Osm(e.to_string());
    let mut ways = Vec::new();
    ElementReader::from_path(pbf)
        .map_err(osm)?
        .for_each(|el| {
            if let Element::Way(w) = el {
                let tags: HashMap<String, String> = w.tags().map(|(k, v)| (k.to_string(), v.to_string())).collect();
                if let Some(speed) = car_speed(&tags) {
                    ways.push(RoadWay { nodes: w.refs().collect(), speed_kmh: speed, oneway: oneway(&tags) });
                }
            }
        })
        .map_err(osm)?;
    let mut needed: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for w in &ways {
        needed.extend(w.nodes.iter().copied());
    }
    let mut coords: HashMap<i64, [f64; 2]> = HashMap::with_capacity(needed.len());
    ElementReader::from_path(pbf)
        .map_err(osm)?
        .for_each(|el| match el {
            Element::DenseNode(n) if needed.contains(&n.id()) => {
                coords.insert(n.id(), [n.lat(), n.lon()]);
            }
            Element::Node(n) if needed.contains(&n.id()) => {
                coords.insert(n.id(), [n.lat(), n.lon()]);
            }
            _ => {}
        })
        .map_err(osm)?;
    drop(needed);
    build(&ways, &coords, out)
}

// ---------- run time ----------

pub struct Router {
    fg: fast_paths::FastGraph,
    nodes: Vec<[f32; 2]>,
    edges: Vec<EdgeRec>,
    geom: Vec<[f32; 2]>,
    grid: HashMap<(i32, i32), Vec<u32>>,
    calc: Mutex<fast_paths::PathCalculator>,
    pub meta: Meta,
}

impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router").field("meta", &self.meta).finish_non_exhaustive()
    }
}

fn cell(lat: f64, lon: f64) -> (i32, i32) {
    ((lat / CELL).floor() as i32, (lon / CELL).floor() as i32)
}

impl Router {
    pub fn load(dir: &Path) -> Result<Self, RoutingError> {
        let fg = read_gz::<Graph32>(&dir.join(GRAPH))?.fg;
        let nodes: Vec<[f32; 2]> = read_gz(&dir.join(NODES))?;
        let edges: Vec<EdgeRec> = read_gz(&dir.join(EDGES))?;
        let geom: Vec<[f32; 2]> = read_gz(&dir.join(GEOM))?;
        let meta: Meta = serde_json::from_slice(&std::fs::read(dir.join(META))?)?;
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, n) in nodes.iter().enumerate() {
            grid.entry(cell(n[0] as f64, n[1] as f64)).or_default().push(i as u32);
        }
        let calc = Mutex::new(fast_paths::create_calculator(&fg));
        Ok(Self { fg, nodes, edges, geom, grid, calc, meta })
    }

    /// Nearest junction within about 5 km.
    pub fn nearest(&self, lat: f64, lon: f64) -> Option<(u32, f64)> {
        let (cy, cx) = cell(lat, lon);
        let mut best: Option<(u32, f64)> = None;
        for r in 0..=5i32 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dy.abs() != r && dx.abs() != r {
                        continue; // only the ring at distance r
                    }
                    if let Some(ids) = self.grid.get(&(cy + dy, cx + dx)) {
                        for &id in ids {
                            let n = self.nodes[id as usize];
                            let d = haversine_m([lat, lon], [n[0] as f64, n[1] as f64]);
                            if best.is_none_or(|(_, bd)| d < bd) {
                                best = Some((id, d));
                            }
                        }
                    }
                }
            }
            // Search one ring past the first hit so a closer node across a cell edge isn't missed.
            if best.is_some() && r >= 1 {
                break;
            }
        }
        best
    }

    fn edge(&self, from: u32, to: u32) -> Option<&EdgeRec> {
        let i = self.edges.partition_point(|e| (e.from, e.to) < (from, to));
        self.edges.get(i).filter(|e| e.from == from && e.to == to)
    }

    /// Route between two [lat, lon] points.
    pub fn route(&self, from: [f64; 2], to: [f64; 2]) -> Option<Route> {
        let (a, da) = self.nearest(from[0], from[1])?;
        let (b, db) = self.nearest(to[0], to[1])?;
        let path = {
            let mut calc = self.calc.lock().ok()?;
            calc.calc_path(&self.fg, a as usize, b as usize)?
        };
        let ids = path.get_nodes();
        let mut coords: Vec<[f64; 2]> = vec![[from[1], from[0]]];
        let (mut dist, mut time) = (0.0f64, 0.0f64);
        if ids.len() == 1 {
            let n = self.nodes[a as usize];
            coords.push([n[1] as f64, n[0] as f64]);
        }
        for w in ids.windows(2) {
            let e = self.edge(w[0] as u32, w[1] as u32)?;
            dist += e.dist_m as f64;
            time += e.time_s as f64;
            let seg = &self.geom[e.geom_off as usize..(e.geom_off + e.geom_len) as usize];
            let mut pts: Vec<[f64; 2]> = seg.iter().map(|p| [p[1] as f64, p[0] as f64]).collect();
            if e.reversed {
                pts.reverse();
            }
            coords.extend(pts);
        }
        coords.push([to[1], to[0]]);
        coords.dedup();
        Some(Route {
            distance_m: dist + da + db,
            duration_s: time,
            coordinates: coords,
            snap_from_m: da,
            snap_to_m: db,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn tag_rules() {
        assert_eq!(car_speed(&tags(&[("highway", "footway")])), None);
        assert_eq!(car_speed(&tags(&[("highway", "residential"), ("access", "private")])), None);
        assert_eq!(car_speed(&tags(&[("highway", "service"), ("service", "driveway")])), None);
        assert_eq!(car_speed(&tags(&[("highway", "primary")])), Some(80.0));
        let s = car_speed(&tags(&[("highway", "primary"), ("maxspeed", "65 mph")])).unwrap();
        assert!((s - 65.0 * 1.609 * 0.9).abs() < 0.1);
        assert_eq!(oneway(&tags(&[("highway", "motorway")])), 1);
        assert_eq!(oneway(&tags(&[("highway", "primary"), ("junction", "roundabout")])), 1);
        assert_eq!(oneway(&tags(&[("highway", "primary"), ("oneway", "-1")])), -1);
        assert_eq!(oneway(&tags(&[("highway", "primary")])), 0);
    }

    /// A 3x3 street grid (~1 km spacing), a one-way shortcut and a disconnected island.
    fn grid_network() -> (Vec<RoadWay>, HashMap<i64, [f64; 2]>) {
        let id = |r: i64, c: i64| r * 10 + c;
        let mut coords = HashMap::new();
        for r in 0..3 {
            for c in 0..3 {
                coords.insert(id(r, c), [30.0 + r as f64 * 0.01, -97.0 + c as f64 * 0.01]);
            }
        }
        coords.insert(1000, [30.0, -96.995]); // mid-block shape point
        coords.insert(900, [31.0, -96.0]);
        coords.insert(901, [31.001, -96.0]);
        let mut ways = Vec::new();
        for r in 0..3 {
            let nodes =
                if r == 0 { vec![id(0, 0), 1000, id(0, 1), id(0, 2)] } else { vec![id(r, 0), id(r, 1), id(r, 2)] };
            ways.push(RoadWay { nodes, speed_kmh: 40.0, oneway: 0 });
        }
        for c in 0..3 {
            ways.push(RoadWay { nodes: vec![id(0, c), id(1, c), id(2, c)], speed_kmh: 40.0, oneway: 0 });
        }
        ways.push(RoadWay { nodes: vec![id(0, 0), id(2, 2)], speed_kmh: 120.0, oneway: 1 });
        ways.push(RoadWay { nodes: vec![900, 901], speed_kmh: 40.0, oneway: 0 });
        (ways, coords)
    }

    #[test]
    fn build_and_route() {
        let (ways, coords) = grid_network();
        let dir = tempfile::tempdir().unwrap();
        let meta = build(&ways, &coords, dir.path()).unwrap();
        assert_eq!(meta.nodes, 9, "shape point is not a junction; island dropped");
        assert_eq!(meta.dropped_nodes, 2);
        let r = Router::load(dir.path()).unwrap();

        let fwd = r.route([30.0, -97.0], [30.02, -96.98]).unwrap();
        let back = r.route([30.02, -96.98], [30.0, -97.0]).unwrap();
        assert!(fwd.duration_s < back.duration_s, "the one-way shortcut only helps forward");
        assert!(back.distance_m > 4000.0 && back.distance_m < 5000.0, "{}", back.distance_m);

        let along = r.route([30.0, -97.0], [30.0, -96.99]).unwrap();
        assert!(along.coordinates.iter().any(|c| (c[0] + 96.995).abs() < 1e-4), "{:?}", along.coordinates);

        let snapped = r.route([30.0005, -97.0005], [30.01, -96.99]).unwrap();
        assert!(snapped.snap_from_m < 100.0);

        assert!(r.nearest(10.0, 10.0).is_none());
        assert!(r.route([10.0, 10.0], [30.0, -97.0]).is_none());
    }
}
