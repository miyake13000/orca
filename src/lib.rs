#[macro_use]
extern crate clap;
extern crate nix;
extern crate reqwest;
extern crate serde;
extern crate serde_json;
extern crate dirs;

pub mod args;
pub mod container;
pub mod image;

const STACK_SIZE: usize = 1024*1024;
