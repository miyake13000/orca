//! orca library: the algorithms tying `orca-vcs`, `orca-image` and
//! `orca-container` together, exposed as a graph of domain objects.
//!
//! Entry and navigation (DESIGN §7):
//!
//! - [`Orca`] — one data root; the only place the root path enters.
//!   Hands out the two independent area stores:
//! - [`EnvStore`] — `envs/` + `run/`: environment CRUD and selection.
//!   Creating an env writes its initial commit graph; deleting checks
//!   for a running container.
//! - [`ExternalImageStore`] — `images/`: pulled OCI images (index, blob
//!   CAS, pull / remove).
//! - [`Env`] — one environment's record and paths; [`Env::image`] opens:
//! - [`Image`] — the env's versioned content (upper + committed layers +
//!   commit graph) with every operation on it: `run`, version control
//!   (commit / checkout / reset / branch / rebase / gc / log), `diff`,
//!   and [`Image::plan_apply`] → [`ApplyPlan`] (two-phase apply:
//!   what the user confirms is exactly what gets executed).
//!
//! The binary (`main.rs` + `cli/`) stays a thin adapter: parse args,
//! navigate this object graph, run prompts, format output, and map
//! errors to exit codes. It only uses this crate's API (types it needs
//! from the lower crates are re-exported below). The library performs no
//! terminal I/O; wherever a confirmation or progress message is needed,
//! the API is split at that point (`find` → `pull`, plan → `execute`).
//! On-disk file names and layouts are decided in this crate, never in
//! the lower crates or the binary.

#![warn(missing_docs)]

mod apply;
mod env;
mod exec_spec;
mod external_image_store;
mod image;
mod lock;
mod orca;
mod run;

pub use env::{Env, EnvError, EnvSettings, EnvStore, orca_root};
pub use external_image_store::{ExternalImageError, ExternalImageStore};
pub use image::{ApplyPlan, ApplyRecovery, Image, ImageError, is_setuid_source};
pub use orca::Orca;
pub use run::{RunError, RunOpts};

// Re-exports for the CLI (and other frontends), so they can format and
// construct values without depending on the lower crates directly.
pub use orca_hash::{Hash, to_hex};
pub use orca_image::external_image::ImageManifest;
pub use orca_image::{BaseImageRef, Change};
pub use orca_vcs::Head;
