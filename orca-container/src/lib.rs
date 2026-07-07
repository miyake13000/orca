//! OverlayFS container runtime for orca.
//!
//! This crate is pure mechanism: it mounts an [`orca_image::Image`] as an
//! overlay rootfs inside a new mount namespace, `pivot_root`s into it and
//! executes a command, optionally wired to the calling terminal through a
//! PTY. Policy — which environment to run, locking, committing — lives in
//! the `orca` crate. In particular the run lock is *not* handled here
//! (`Container` never touches `run/<uuid>/lock`).
//!
//! Entry points: [`ContainerBuilder`] → [`Container<Created>::run`] →
//! [`Container<Running>::wait`] (typestate pattern).

#![warn(missing_docs)]

mod container;
mod image;
pub mod mount;

pub use container::{
    Container, ContainerBuilder, ContainerError, Created, IoMode, NamespaceOpts, RunAs, Running,
    Terminated,
};
pub use image::{OverlayMount, Rootfs, SessionPaths};

/// Stack size handed to `clone(2)` for the container child (1 MiB).
pub const STACK_SIZE: usize = 1024 * 1024;
