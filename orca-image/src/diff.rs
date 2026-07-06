//! Read-only diff between two layer stacks: [`changes`] and [`Change`].
//!
//! `changes(new, base)` walks the merged view of the `new` stack and looks
//! each path up in the `base` stack, emitting `Create` / `Modify` /
//! `Delete`. Both stacks are ordered top (highest priority) to bottom; the
//! caller resolves commit hashes into stacks and decides what the bottom
//! layer is (the host `/` or extracted image layers) — see DESIGN §8:
//!
//! ```text
//! apply (all changes):   changes(upper+committed, base=[rootdir])
//! apply --no-upper:      changes(committed,       base=[rootdir])
//! orca diff:             changes([upper],         base=lower+base-image)
//! orca diff A / A B:     changes(stack_of(A),     base=base / stack_of(B))
//! ```
//!
//! The computation is read-only (the host is only ever stat'ed / listed)
//! and existence-based: file contents are never compared. Whiteouts and
//! opaque markers are interpreted in the OverlayFS native format only
//! (0:0 char devices and the `trusted.overlay.opaque` xattr, see the
//! crate-private `whiteout` module); opaque directories are expanded into
//! individual `Delete`s against the base here, so consumers never need to
//! interpret layer semantics or walk the host themselves.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::layer::Layer;
use crate::whiteout;

/// Errors from diff computation.
#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    /// Reading a layer directory or stat'ing an entry failed.
    #[error("failed to read {path} while computing diff: {source}")]
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

/// One file-level change of the merged `new` stack relative to `base`.
///
/// `path` is relative to the filesystem root (no leading `/`); `source` is
/// the absolute path of the providing entry inside its layer directory.
/// Serializable so `orca apply` can persist its manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    /// The path does not exist in `base`: a new file / directory / symlink.
    Create {
        /// Path relative to the filesystem root.
        path: PathBuf,
        /// Absolute path of the content inside the providing layer.
        source: PathBuf,
    },
    /// The path exists in `base` and is overwritten.
    Modify {
        /// Path relative to the filesystem root.
        path: PathBuf,
        /// Absolute path of the content inside the providing layer.
        source: PathBuf,
    },
    /// The path exists in `base` and is deleted (whiteout / opaque).
    Delete {
        /// Path relative to the filesystem root.
        path: PathBuf,
    },
}

impl Change {
    /// The affected path (relative to the filesystem root).
    pub fn path(&self) -> &Path {
        match self {
            Change::Create { path, .. } | Change::Modify { path, .. } | Change::Delete { path } => {
                path
            }
        }
    }

    /// The providing source path, if the change carries content.
    pub fn source(&self) -> Option<&Path> {
        match self {
            Change::Create { source, .. } | Change::Modify { source, .. } => Some(source),
            Change::Delete { .. } => None,
        }
    }
}

/// A node in the merged view of the `new` stack.
#[derive(Debug)]
enum Node {
    /// Deletion marker (0:0 char device) provided by `layer`.
    Whiteout { layer: usize },
    /// A visible entry provided by `layer`.
    Entry {
        source: PathBuf,
        is_dir: bool,
        opaque: bool,
        layer: usize,
    },
}

/// Compute the changes of the merged `new` stack relative to `base`.
///
/// Both slices are ordered top to bottom. Directories that already exist
/// in `base` are treated as merged (no `Modify` is emitted for them);
/// metadata-only changes to existing directories are therefore not
/// captured. Results are ordered lexicographically, except that the
/// `Delete`s expanded from an opaque directory are emitted deepest-first.
pub fn changes(new: &[Layer], base: &[Layer]) -> Result<Vec<Change>, DiffError> {
    let merged = merge_new(new)?;
    let mut out = Vec::new();
    for (rel, node) in &merged {
        match node {
            Node::Whiteout { .. } => {
                if lookup_base(base, rel)? {
                    out.push(Change::Delete { path: rel.clone() });
                }
            }
            Node::Entry {
                source,
                is_dir,
                opaque,
                ..
            } => {
                let in_base = lookup_base(base, rel)?;
                if *is_dir {
                    if !in_base {
                        out.push(Change::Create {
                            path: rel.clone(),
                            source: source.clone(),
                        });
                    }
                    if *opaque && in_base {
                        // Everything below this directory in `base` is cut
                        // off; delete whatever the new stack does not
                        // provide, deepest-first.
                        let mut deletes: Vec<PathBuf> = list_base_under(base, rel)?
                            .into_iter()
                            .filter(|p| !matches!(merged.get(p), Some(Node::Entry { .. })))
                            .collect();
                        deletes.sort();
                        deletes.reverse();
                        out.extend(deletes.into_iter().map(|path| Change::Delete { path }));
                    }
                } else if in_base {
                    out.push(Change::Modify {
                        path: rel.clone(),
                        source: source.clone(),
                    });
                } else {
                    out.push(Change::Create {
                        path: rel.clone(),
                        source: source.clone(),
                    });
                }
            }
        }
    }
    Ok(out)
}

/// Build the merged view of the `new` stack: for every visible relative
/// path, the winning node (higher layers win; whiteouts, opaque dirs and
/// non-directories shadow lower layers).
fn merge_new(new: &[Layer]) -> Result<BTreeMap<PathBuf, Node>, DiffError> {
    let mut merged: BTreeMap<PathBuf, Node> = BTreeMap::new();
    for (idx, layer) in new.iter().enumerate() {
        let walker = walkdir::WalkDir::new(layer.path())
            .min_depth(1)
            .follow_links(false)
            .sort_by_file_name();
        for entry in walker {
            let entry = entry.map_err(|e| walk_err(layer.path(), e))?;
            let rel = entry
                .path()
                .strip_prefix(layer.path())
                .expect("walkdir yields children of the layer root")
                .to_path_buf();
            if merged.contains_key(&rel) || shadowed_by_ancestor(&merged, &rel, idx) {
                continue;
            }
            let md = entry.metadata().map_err(|e| walk_err(entry.path(), e))?;
            let node = if whiteout::is_whiteout(&md) {
                Node::Whiteout { layer: idx }
            } else {
                let is_dir = md.is_dir();
                Node::Entry {
                    source: entry.path().to_path_buf(),
                    is_dir,
                    opaque: is_dir && whiteout::is_opaque(entry.path()),
                    layer: idx,
                }
            };
            merged.insert(rel, node);
        }
    }
    Ok(merged)
}

/// Whether an entry of layer `layer_idx` at `rel` is hidden by an ancestor
/// recorded in `merged` from a strictly higher layer: a whiteout, an
/// opaque directory, or a non-directory (all of which cut off lower
/// layers). Non-opaque directories merge and do not shadow.
fn shadowed_by_ancestor(merged: &BTreeMap<PathBuf, Node>, rel: &Path, layer_idx: usize) -> bool {
    let mut ancestor = rel.parent();
    while let Some(anc) = ancestor {
        if anc.as_os_str().is_empty() {
            break;
        }
        match merged.get(anc) {
            Some(Node::Whiteout { layer }) if *layer < layer_idx => return true,
            Some(Node::Entry {
                is_dir, opaque, layer, ..
            }) if *layer < layer_idx && (!*is_dir || *opaque) => return true,
            _ => {}
        }
        ancestor = anc.parent();
    }
    false
}

/// Overlay-style lookup: whether `rel` exists (as a visible entry) in the
/// `base` stack.
fn lookup_base(base: &[Layer], rel: &Path) -> Result<bool, DiffError> {
    for layer in base {
        // Check ancestors within this layer: a whiteout, an opaque
        // directory or a non-directory on the way cuts off lower layers.
        let mut opaque_cut = false;
        let mut blocked = false;
        let ancestors: Vec<&Path> = {
            let mut v: Vec<&Path> = std::iter::successors(rel.parent(), |p| p.parent())
                .take_while(|p| !p.as_os_str().is_empty())
                .collect();
            v.reverse();
            v
        };
        for anc in ancestors {
            let full = layer.path().join(anc);
            match std::fs::symlink_metadata(&full) {
                Ok(md) if whiteout::is_whiteout(&md) => return Ok(false),
                Ok(md) if !md.is_dir() => return Ok(false),
                Ok(_) => {
                    if whiteout::is_opaque(&full) {
                        opaque_cut = true;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    blocked = true;
                    break;
                }
                Err(e) => return Err(io_err(&full, e)),
            }
        }
        if blocked {
            if opaque_cut {
                return Ok(false);
            }
            continue;
        }
        let full = layer.path().join(rel);
        match std::fs::symlink_metadata(&full) {
            Ok(md) => return Ok(!whiteout::is_whiteout(&md)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if opaque_cut {
                    return Ok(false);
                }
            }
            Err(e) => return Err(io_err(&full, e)),
        }
    }
    Ok(false)
}

/// Recursively list the merged `base` view under directory `rel`
/// (which must exist as a directory in the merged base). Used to expand an
/// opaque directory into individual deletes; this is where the host may be
/// walked (read-only).
fn list_base_under(base: &[Layer], rel: &Path) -> Result<Vec<PathBuf>, DiffError> {
    let mut out = Vec::new();
    let mut dirs = vec![rel.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        // name -> is_dir, first (topmost) layer wins; whiteouts hide.
        let mut names: BTreeMap<std::ffi::OsString, Option<bool>> = BTreeMap::new();
        for layer in base {
            let full = layer.path().join(&dir);
            let md = match std::fs::symlink_metadata(&full) {
                Ok(md) => md,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io_err(&full, e)),
            };
            if whiteout::is_whiteout(&md) || !md.is_dir() {
                break; // cuts off lower layers
            }
            let entries = std::fs::read_dir(&full).map_err(|e| io_err(&full, e))?;
            for entry in entries {
                let entry = entry.map_err(|e| io_err(&full, e))?;
                let name = entry.file_name();
                if names.contains_key(&name) {
                    continue;
                }
                let emd = entry.path().symlink_metadata().map_err(|e| io_err(&entry.path(), e))?;
                let value = if whiteout::is_whiteout(&emd) {
                    None
                } else {
                    Some(emd.is_dir())
                };
                names.insert(name, value);
            }
            if whiteout::is_opaque(&full) {
                break; // opaque: lower layers cut off
            }
        }
        for (name, kind) in names {
            let Some(is_dir) = kind else { continue };
            let child = dir.join(&name);
            if is_dir {
                dirs.push(child.clone());
            }
            out.push(child);
        }
    }
    Ok(out)
}

fn walk_err(path: &Path, e: walkdir::Error) -> DiffError {
    DiffError::Io {
        path: e
            .path()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf()),
        source: e
            .into_io_error()
            .unwrap_or_else(|| std::io::Error::other("walkdir loop")),
    }
}

fn io_err(path: &Path, e: std::io::Error) -> DiffError {
    DiffError::Io {
        path: path.to_path_buf(),
        source: e,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(dir: &Path) -> Layer {
        Layer::new(dir.to_path_buf())
    }

    fn touch(dir: &Path, rel: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    #[test]
    fn create_and_modify_against_base() {
        let new = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        touch(new.path(), "etc/new.conf");
        touch(new.path(), "etc/existing.conf");
        touch(base.path(), "etc/existing.conf");

        let got = changes(&[layer(new.path())], &[layer(base.path())]).unwrap();
        assert_eq!(
            got,
            vec![
                Change::Modify {
                    path: "etc/existing.conf".into(),
                    source: new.path().join("etc/existing.conf"),
                },
                Change::Create {
                    path: "etc/new.conf".into(),
                    source: new.path().join("etc/new.conf"),
                },
            ]
        );
    }

    #[test]
    fn new_directory_is_created_existing_is_merged() {
        let new = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        touch(new.path(), "opt/app/bin");
        std::fs::create_dir_all(base.path().join("opt")).unwrap();

        let got = changes(&[layer(new.path())], &[layer(base.path())]).unwrap();
        // "opt" exists in base -> merged silently; "opt/app" and below are new.
        assert_eq!(
            got,
            vec![
                Change::Create {
                    path: "opt/app".into(),
                    source: new.path().join("opt/app"),
                },
                Change::Create {
                    path: "opt/app/bin".into(),
                    source: new.path().join("opt/app/bin"),
                },
            ]
        );
    }

    #[test]
    fn upper_layer_wins_over_lower() {
        let top = tempfile::tempdir().unwrap();
        let bottom = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        touch(top.path(), "etc/conf");
        touch(bottom.path(), "etc/conf");
        touch(bottom.path(), "etc/other");

        let got = changes(&[layer(top.path()), layer(bottom.path())], &[layer(base.path())])
            .unwrap();
        assert_eq!(
            got,
            vec![
                Change::Create {
                    path: "etc".into(),
                    source: top.path().join("etc"),
                },
                Change::Create {
                    path: "etc/conf".into(),
                    source: top.path().join("etc/conf"),
                },
                Change::Create {
                    path: "etc/other".into(),
                    source: bottom.path().join("etc/other"),
                },
            ]
        );
    }

    #[test]
    fn file_shadows_lower_directory() {
        let top = tempfile::tempdir().unwrap();
        let bottom = tempfile::tempdir().unwrap();
        touch(top.path(), "data"); // file
        touch(bottom.path(), "data/inner"); // dir with file below

        let got = changes(&[layer(top.path()), layer(bottom.path())], &[]).unwrap();
        assert_eq!(
            got,
            vec![Change::Create {
                path: "data".into(),
                source: top.path().join("data"),
            }]
        );
    }

    #[test]
    fn symlink_is_a_change_carrier() {
        let new = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("/target", new.path().join("link")).unwrap();
        let got = changes(&[layer(new.path())], &[layer(base.path())]).unwrap();
        assert_eq!(
            got,
            vec![Change::Create {
                path: "link".into(),
                source: new.path().join("link"),
            }]
        );
    }

    /// Whiteout / opaque tests need mknod + trusted xattrs -> root only.
    /// Run with: sudo -E cargo test -p orca-image -- --ignored
    #[test]
    #[ignore = "requires root (mknod / trusted.* xattr)"]
    fn whiteout_becomes_delete_and_opaque_expands() {
        let new = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();

        // Whiteout on a file that exists in base -> Delete.
        touch(base.path(), "etc/remove.me");
        std::fs::create_dir_all(new.path().join("etc")).unwrap();
        whiteout::make_whiteout(&new.path().join("etc/remove.me")).unwrap();

        // Whiteout on a file that does not exist in base -> nothing.
        whiteout::make_whiteout(&new.path().join("etc/ghost")).unwrap();

        // Opaque dir: base contents disappear except what new provides.
        touch(base.path(), "opt/app/keep");
        touch(base.path(), "opt/app/drop/deep");
        touch(new.path(), "opt/app/keep");
        whiteout::set_opaque(&new.path().join("opt/app")).unwrap();

        let got = changes(&[layer(new.path())], &[layer(base.path())]).unwrap();
        assert!(got.contains(&Change::Delete { path: "etc/remove.me".into() }));
        assert!(!got.iter().any(|c| c.path() == Path::new("etc/ghost")));
        assert!(got.contains(&Change::Delete { path: "opt/app/drop/deep".into() }));
        assert!(got.contains(&Change::Delete { path: "opt/app/drop".into() }));
        assert!(got.contains(&Change::Modify {
            path: "opt/app/keep".into(),
            source: new.path().join("opt/app/keep"),
        }));
        // Deepest-first among the opaque deletes.
        let drop_pos = got
            .iter()
            .position(|c| c.path() == Path::new("opt/app/drop"))
            .unwrap();
        let deep_pos = got
            .iter()
            .position(|c| c.path() == Path::new("opt/app/drop/deep"))
            .unwrap();
        assert!(deep_pos < drop_pos);
    }
}
