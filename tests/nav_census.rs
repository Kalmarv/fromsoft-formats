//! Every navmesh binder the installs hold (`*.nvmhktbnd.dcx`), through [`havok_formats::navmesh::nav_data`]: every
//! member tagfile reads (each carries its own type table), and the totals are those of the census that found them
//! with its own reading of the same files (Sekiro: 10 binders, 1,509 tagfiles, 279,053 faces, 1,160,836 edges; Elden
//! Ring's base game: 695 binders, 7,438 tagfiles, 6,317,094 faces, 26,313,941 edges). With Shadow of the Erdtree
//! installed Elden Ring's are this test's own (997 binders, 10,880 tagfiles, 9,476,723 faces, 39,262,850 edges); with
//! the expansion's archives left out it gives the census's.
//!
//! The binders are found from the games' MSB map names as well as from the dictionaries: a binder either archive
//! holds that its dictionary does not name is read too (the test counts them; there are none).
//!
//! Needs `installs.{sekiro,elden}`, `keys.{sekiro,elden}`, `tools.oodle6` and
//! `oracles.uxm.{sekiro,elden}/dictionary.txt`; a game that is not configured is skipped.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod common;
mod inputs;

use fromsoft_formats::dcx::Decoder;
use fromsoft_formats::{Archive, Game, NameDictionary};
use havok_formats::Tagfile;
use havok_formats::navmesh::nav_data;
use inputs::{Game as Data, OracleSet, Tool, oodle, oracle};
use std::collections::BTreeSet;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// What the navmesh binders of one game hold.
#[derive(Debug, Default, PartialEq, Eq)]
struct Totals {
    binders: usize,
    /// Binders no dictionary path names.
    unnamed_binders: usize,
    files: usize,
    /// Files by the SDK version string of their tagfile.
    sdk_2016: usize,
    sdk_2018: usize,
    faces: usize,
    edges: usize,
    user_edges: usize,
    vertices: usize,
    clustered_faces: usize,
    /// Meshes with no faces.
    empty_meshes: usize,
    triangles: usize,
    incomplete_faces: usize,
    graphs: usize,
    graph_nodes: usize,
    graph_edges: usize,
    trees: usize,
    tree_nodes: usize,
    setup_arrays: usize,
    setups: usize,
    /// Graphs that are the cluster graph of the mesh in their file (`NavData::graph_is_cluster_graph`).
    graphs_that_are_cluster_graphs: usize,
}

impl Totals {
    /// Adds the counts of other files to these.
    fn add(&mut self, other: Totals) {
        self.binders += other.binders;
        self.unnamed_binders += other.unnamed_binders;
        self.files += other.files;
        self.sdk_2016 += other.sdk_2016;
        self.sdk_2018 += other.sdk_2018;
        self.faces += other.faces;
        self.edges += other.edges;
        self.user_edges += other.user_edges;
        self.vertices += other.vertices;
        self.clustered_faces += other.clustered_faces;
        self.empty_meshes += other.empty_meshes;
        self.triangles += other.triangles;
        self.incomplete_faces += other.incomplete_faces;
        self.graphs += other.graphs;
        self.graph_nodes += other.graph_nodes;
        self.graph_edges += other.graph_edges;
        self.trees += other.trees;
        self.tree_nodes += other.tree_nodes;
        self.setup_arrays += other.setup_arrays;
        self.setups += other.setups;
        self.graphs_that_are_cluster_graphs += other.graphs_that_are_cluster_graphs;
    }
}

/// The paths of the navmesh binders the game's maps would have, and how many its dictionary does not name.
fn binders(game: Game, archive: &Archive, names: &NameDictionary) -> (Vec<String>, usize) {
    let dictionary: BTreeSet<&str> = names
        .names
        .values()
        .flatten()
        .map(String::as_str)
        .filter(|p| p.starts_with("/map/") && p.ends_with(".nvmhktbnd.dcx"))
        .filter(|p| archive.contains(p))
        .collect();
    let mut all = dictionary.clone();
    let maps: BTreeSet<&str> = names
        .names
        .values()
        .flatten()
        .filter_map(|p| p.strip_prefix("/map/mapstudio/")?.strip_suffix(".msb.dcx"))
        .filter(|m| !m.contains('/'))
        .collect();
    let derived: Vec<String> = maps
        .iter()
        .flat_map(|map| {
            let flat = format!("/map/{map}/{map}.nvmhktbnd.dcx");
            let tiled = format!("/map/{}/{map}/{map}.nvmhktbnd.dcx", &map[..3]);
            match game {
                Game::Sekiro => vec![flat],
                Game::EldenRing => vec![tiled, flat],
            }
        })
        .filter(|p| archive.contains(p))
        .collect();
    all.extend(derived.iter().map(String::as_str));
    let unnamed = all.len() - dictionary.len();
    (all.into_iter().map(str::to_owned).collect(), unnamed)
}

fn totals(game: Game) -> Result<Option<Totals>> {
    let data = match game {
        Game::Sekiro => Data::Sekiro,
        Game::EldenRing => Data::Elden,
    };
    let (Some(archive), Some(dll), Some(dictionary)) = (
        common::archive(game)?,
        oodle(Tool::Oodle6),
        oracle(OracleSet::Uxm, data, "dictionary.txt"),
    ) else {
        return Ok(None);
    };
    let dll = common::oodle(dll)?;
    let decoder = Decoder {
        oodle: Some(&dll),
        max_output: 1 << 30,
    };
    let names = NameDictionary::read_for(dictionary, game)?;
    let (paths, unnamed) = binders(game, &archive, &names);
    let mut t = Totals {
        binders: paths.len(),
        unnamed_binders: unnamed,
        ..Totals::default()
    };
    // each binder's totals, read on the archive's threads and added up in path order
    let binders = archive.read_each_binder(
        &paths,
        &decoder,
        inputs::threads(),
        |path, binder| -> Result<Totals> {
            let mut t = Totals::default();
            for f in &binder?.files {
                let name = f.name.as_deref().unwrap_or("");
                let bytes = decoder.decode_if_dcx(&f.bytes)?;
                if !name.ends_with(".hkx") {
                    return Err(format!("{path}: {name}: a member that is not a .hkx tagfile").into());
                }
                let tag = Tagfile::read(&bytes, None).map_err(|e| format!("{path}: {name}: {e}"))?;
                let nav = nav_data(tag.root()?).map_err(|e| format!("{path}: {name}: {e}"))?;
                match &tag.version {
                    b"20160200" => t.sdk_2016 += 1,
                    b"20180100" => t.sdk_2018 += 1,
                    other => return Err(format!("{path}: {name}: SDK {other:?}").into()),
                }
                t.files += 1;
                let mesh = &nav.mesh;
                t.faces += mesh.faces.len();
                t.edges += mesh.edges.len();
                t.user_edges += mesh.edges.iter().filter(|e| e.user).count();
                t.vertices += mesh.vertices.len();
                t.clustered_faces += mesh.faces.iter().filter(|f| f.cluster.is_some()).count();
                t.empty_meshes += usize::from(mesh.faces.is_empty());
                for face in 0..mesh.faces.len() {
                    let cut = mesh.triangulate(face).ok_or("a face that does not triangulate")?;
                    t.triangles += cut.triangles.len();
                    t.incomplete_faces += usize::from(!cut.complete);
                }
                if let Some(graph) = &nav.graph {
                    t.graphs += 1;
                    t.graph_nodes += graph.nodes.len();
                    t.graph_edges += graph.edges.len();
                    t.graphs_that_are_cluster_graphs += usize::from(nav.graph_is_cluster_graph());
                }
                if let Some(tree) = &nav.tree {
                    t.trees += 1;
                    t.tree_nodes += tree.nodes.len();
                    assert!(tree.covers_faces_once(mesh.faces.len()), "{path}: {name}");
                    assert!(tree.corner_excess(mesh) <= 1e-3, "{path}: {name}");
                }
                if let Some(setups) = &nav.user_edge_setups {
                    t.setup_arrays += 1;
                    t.setups += setups.len();
                }
            }
            Ok(t)
        },
    )?;
    for binder in binders {
        t.add(binder?);
    }
    eprintln!("{} navmesh binders: {t:?}", game.name());
    Ok(Some(t))
}

#[test]
fn sekiros_navmesh_binders_read_with_the_census_totals() -> Result<()> {
    let Some(t) = totals(Game::Sekiro)? else {
        return Ok(());
    };
    assert_eq!((t.binders, t.unnamed_binders, t.files), (10, 0, 1_509));
    assert_eq!((t.sdk_2016, t.sdk_2018), (1_509, 0));
    assert_eq!((t.faces, t.edges, t.vertices), (279_053, 1_160_836, 454_816));
    assert_eq!(
        (t.user_edges, t.clustered_faces, t.empty_meshes),
        (0, 279_053, 94)
    );
    // havok-formats 0.9.0 keeps the corners straight only seen from above (208 triangles); its census checks every
    // face's triangles against the polygon's area vector
    assert_eq!((t.triangles, t.incomplete_faces), (602_658, 0));
    assert_eq!(
        (
            t.graphs,
            t.graph_nodes,
            t.graph_edges,
            t.graphs_that_are_cluster_graphs
        ),
        (1_509, 20_803, 52_200, 1_509)
    );
    assert_eq!((t.trees, t.tree_nodes), (0, 0));
    assert_eq!((t.setup_arrays, t.setups), (430, 1_463));
    Ok(())
}

#[test]
fn elden_rings_navmesh_binders_read_with_the_census_totals() -> Result<()> {
    let Some(t) = totals(Game::EldenRing)? else {
        return Ok(());
    };
    assert_eq!((t.binders, t.unnamed_binders, t.files), (997, 0, 10_880));
    assert_eq!((t.sdk_2016, t.sdk_2018), (0, 10_880));
    assert_eq!(
        (t.faces, t.edges, t.vertices),
        (9_476_723, 39_262_850, 19_940_555)
    );
    assert_eq!(
        (t.user_edges, t.clustered_faces, t.empty_meshes),
        (3_924_697, 0, 6_316)
    );
    // in the base game 3,826 of them the corners straight only seen from above (havok-formats 0.9.0's census)
    assert_eq!((t.triangles, t.incomplete_faces), (16_381_383, 2));
    assert_eq!((t.graphs, t.graph_nodes, t.graph_edges), (0, 0, 0));
    assert_eq!((t.trees, t.tree_nodes), (8_833, 18_948_882));
    assert_eq!((t.setup_arrays, t.setups), (263, 959));
    Ok(())
}
