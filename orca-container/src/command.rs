use anyhow::Result;
use std::cmp::PartialEq;
use std::ffi::OsStr;
use std::path::Path;
use std::{env, process};

pub struct Command<T, S, U>
where
    T: AsRef<OsStr> + AsRef<Path>,
    S: IntoIterator<Item = U> + PartialEq,
    U: AsRef<OsStr>,
{
    cmd_name: T,
    args: Option<S>,
}

impl<T, S, U> Command<T, S, U>
where
    T: AsRef<OsStr> + AsRef<Path>,
    S: IntoIterator<Item = U> + PartialEq,
    U: AsRef<OsStr>,
{
    pub fn new(cmd_name: T, args: Option<S>) -> Self {
        Self { cmd_name, args }
    }

    pub fn is_exist(&self) -> bool {
        let command_path = env::var_os("PATH").and_then(|paths| {
            env::split_paths(&paths).find_map(|dir| {
                let full_path = dir.join(&self.cmd_name);
                if full_path.is_file() {
                    Some(full_path)
                } else {
                    None
                }
            })
        });

        command_path != None
    }

    pub fn execute(self) -> Result<Option<i32>> {
        let mut command = process::Command::new(&self.cmd_name);

        if self.args != None {
            command.args(self.args.unwrap());
        }

        let res = command.status()?;
        Ok(res.code())
    }
}
