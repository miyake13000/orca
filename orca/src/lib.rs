//! orca library: environment management, locking, and the workspace
//! facade tying `orca-vcs`, `orca-image` and `orca-container` together.
//!
//! Responsibility split (DESIGN §7):
//! - [`EnvStore`] / [`Env`] — environment CRUD and path resolution
//!   (`envs.toml`).
//! - [`LockFile`] — the run/vcs shared exclusion mechanism
//!   (`run/<uuid>/lock`); `orca-container` knows nothing about it.
//! - [`Workspace`] — every operation touching history or layers
//!   (commit / checkout / reset / branch / rebase / clean / gc / diff /
//!   apply), with the preconditions (no running container, clean upper)
//!   checked once at the entry points.
//! - [`apply`] — journaled application of [`orca_image::Change`]s to the
//!   host.
//!
//! The binary (`main.rs` + `cli/`) stays a thin adapter: parse args, call
//! the library, format output, map errors to exit codes.

#![warn(missing_docs)]

pub mod apply;
mod env;
mod exec_spec;
mod lock;
mod workspace;

/// The file name to store the commit and metadata
pub const COMMIT_FILE_NAME: &str = "commits.toml";

pub use env::{Env, EnvError, EnvSettings, EnvStore, orca_root};
pub use exec_spec::{ExecSpec, ExecSpecError, Invocation, UserIdentity, resolve_run_target};
pub use lock::{LockError, LockFile};
pub use workspace::{Workspace, WorkspaceError, is_setuid_source};
