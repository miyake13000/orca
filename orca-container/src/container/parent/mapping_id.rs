use anyhow::{Context, Result};
use nix::unistd::{getegid, geteuid};
use std::env;
use std::fmt::Display;
use std::fs::File;
use std::io::{prelude::*, BufReader};

pub struct MappingID {
    source_id: usize,
    target_id: usize,
    range: usize,
}

#[allow(clippy::upper_case_acronyms)]
pub enum IDType {
    UID,
    GID,
    SUBUID,
    SUBGID,
}

impl MappingID {
    pub fn create(idtype: IDType) -> Result<MappingID> {
        match idtype {
            IDType::UID => {
                let uid = get_uid();
                Ok(MappingID {
                    source_id: uid,
                    target_id: 0,
                    range: 1,
                })
            }
            IDType::GID => {
                let gid = get_gid();
                Ok(MappingID {
                    source_id: gid,
                    target_id: 0,
                    range: 1,
                })
            }
            IDType::SUBUID => {
                let subuid = get_subuid().context("Failed to get subuid")?;
                Ok(MappingID {
                    source_id: subuid.0,
                    target_id: 1,
                    range: subuid.1,
                })
            }
            IDType::SUBGID => {
                let subgid = get_subgid().context("Failed to get subgid")?;
                Ok(MappingID {
                    source_id: subgid.0,
                    target_id: 1,
                    range: subgid.1,
                })
            }
        }
    }

    pub fn into_vec(self) -> Vec<String> {
        vec![
            self.target_id.to_string(),
            self.source_id.to_string(),
            self.range.to_string(),
        ]
    }
}

impl Display for MappingID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} {}", self.target_id, self.source_id, self.range)
    }
}

fn get_uid() -> usize {
    geteuid().as_raw() as usize
}

fn get_gid() -> usize {
    getegid().as_raw() as usize
}

fn get_subuid() -> Result<(usize, usize)> {
    let path = "/etc/subuid";
    let file = File::open(path).with_context(|| format!("Failed to open '{}'", path))?;
    let mut reader = BufReader::new(file);
    let mut contents = String::new();
    let _ = reader.read_to_string(&mut contents);

    let username = env::var("USER").context("Failed to get USER from environmental variable")?;
    let user_entry = username + ":";

    let mut subuid: usize = 0;
    let mut range: usize = 0;

    for line in contents.lines() {
        if line.starts_with(&user_entry) {
            let v: Vec<&str> = line.split(':').collect();
            subuid = v[1].parse().context("Invalid subuid format")?;
            range = v[2].parse().context("Invalid subuid format")?;
        }
    }
    Ok((subuid, range))
}

fn get_subgid() -> Result<(usize, usize)> {
    let path = "/etc/subgid";
    let file = File::open(path).with_context(|| format!("Failed to open '{}'", path))?;
    let mut reader = BufReader::new(file);
    let mut contents = String::new();
    let _len = reader.read_to_string(&mut contents);

    let username = env::var("USER").context("Failed to get USER from environmental variable")?;
    let user_entry = username + ":";

    let mut subgid: usize = 0;
    let mut range: usize = 0;

    for line in contents.lines() {
        if line.starts_with(&user_entry) {
            let v: Vec<&str> = line.split(':').collect();
            subgid = v[1].parse().context("Invalid subgid format")?;
            range = v[2].parse().context("Invalid subgid format")?;
        }
    }
    Ok((subgid, range))
}
