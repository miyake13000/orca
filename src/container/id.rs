use anyhow::{Context, Result};
use nix::unistd::{getegid, geteuid};
use std::env;
use std::fs::File;
use std::io::{prelude::*, BufReader};

pub struct MappingId {
    source_id: usize,
    target_id: usize,
    range: usize,
}

pub enum IdType {
    Uid,
    Gid,
    SubUid,
    SubGid,
}

impl MappingId {
    pub fn new(idtype: IdType) -> Result<Self> {
        match idtype {
            IdType::Uid => {
                let uid = Self::get_uid();
                Ok(MappingId {
                    source_id: uid.0,
                    target_id: 0,
                    range: uid.1,
                })
            }
            IdType::Gid => {
                let gid = Self::get_gid();
                Ok(MappingId {
                    source_id: gid.0,
                    target_id: 0,
                    range: gid.1,
                })
            }
            IdType::SubUid => {
                let subuid = Self::get_subuid().context("Failed to get subuid")?;
                Ok(MappingId {
                    source_id: subuid.0,
                    target_id: 1,
                    range: subuid.1,
                })
            }
            IdType::SubGid => {
                let subgid = Self::get_subgid().context("Failed to get subgid")?;
                Ok(MappingId {
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

    pub fn to_string(&self) -> String {
        format!("{} {} {}", self.target_id, self.source_id, self.range)
    }

    fn get_uid() -> (usize, usize) {
        let uid = geteuid().as_raw() as usize;
        let range = 1;
        (uid, range)
    }

    fn get_gid() -> (usize, usize) {
        let gid = getegid().as_raw() as usize;
        let range = 1;
        (gid, range)
    }

    fn get_subuid() -> Result<(usize, usize)> {
        let path = "/etc/subuid";
        let file = File::open(path).with_context(|| format!("Failed to open '{}'", path))?;
        let mut reader = BufReader::new(file);
        let mut contents = String::new();
        let _ = reader.read_to_string(&mut contents);

        let username =
            env::var("USER").context("Failed to get USER from environmental variable")?;
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
        let _ = reader.read_to_string(&mut contents);

        let username =
            env::var("USER").context("Failed to get USER from environmental variable")?;
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
}
