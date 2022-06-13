use anyhow::{Context, Result};
use rm_rf::remove;
use std::fs::copy;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const WHITE_OUT_PREFIX: &str = ".wh.";
const OPAQUE_FILE_NAME: &str = ".wh..wh..opq";

trait IsSame<T, U>
where
    T: PartialEq<U>,
    U: PartialEq<T>,
{
    fn is_same(&self, other: Option<U>) -> bool;
}

impl<T, U> IsSame<T, U> for Option<T>
where
    T: PartialEq<U> + Clone,
    U: PartialEq<T>,
{
    fn is_same(&self, other: Option<U>) -> bool {
        if self.is_some() && other.is_some() {
            self.clone().unwrap() == other.unwrap()
        } else {
            self.is_none() && other.is_none()
        }
    }
}

trait WhiteoutFile {
    fn is_wh_file(&self) -> Option<PathBuf>;
    fn is_opq_file(&self) -> bool;
    fn contains_opq_file(&self) -> bool;
}

impl WhiteoutFile for Path {
    fn is_wh_file(&self) -> Option<PathBuf> {
        let res = self.file_name().map(|file_name| {
            file_name
                .to_str()
                .map(|file_name| file_name.strip_prefix(WHITE_OUT_PREFIX))
        });
        if let Some(Some(Some(file_name))) = res {
            Some(AsRef::<Path>::as_ref(file_name).to_path_buf())
        } else {
            None
        }
    }

    fn is_opq_file(&self) -> bool {
        self.file_name().is_same(Some(OPAQUE_FILE_NAME))
    }

    fn contains_opq_file(&self) -> bool {
        WalkDir::new(self)
            .max_depth(1)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .any(|entry| entry.path().file_name().is_same(Some(OPAQUE_FILE_NAME)))
    }
}

fn copy_layer<T, U>(src: T, dest: U) -> Result<()>
where
    T: AsRef<Path>,
    U: AsRef<Path>,
{
    let src = src.as_ref();
    let dest = dest.as_ref();

    if !dest.is_dir() {
        std::fs::create_dir_all(&dest)?;
    }
    let src_prefix = src
        .canonicalize()
        .with_context(|| format!("Failed to get absolute path: '{}'", src.display()))?;
    let dest_prefix = dest
        .canonicalize()
        .with_context(|| format!("Failed to get absolute path: '{}'", dest.display()))?;

    for entry in WalkDir::new(&src_prefix).into_iter().filter_map(|e| e.ok()) {
        let src = entry.path();
        let rel_dest = &src
            .strip_prefix(&src_prefix)
            .with_context(|| format!("Failed to strip path: '{}'", entry.path().display()))?;
        let dest = &dest_prefix.join(rel_dest);

        if src.is_symlink() {
            if dest.exists() {
                remove(&dest).with_context(|| format!("Failed to remove: '{}'", dest.display()))?;
            }
            copy_symlink(&src, &dest)?;
        } else if src.is_file() {
            if src.is_opq_file() {
                //do nothing
            } else if let Some(file_name) = src.is_wh_file() {
                let dest_to_remove = dest.parent().unwrap().join(file_name);
                remove(dest_to_remove)
                    .with_context(|| format!("Failed to remove: '{}'", dest.display()))?;
            } else {
                copy_file(&src, &dest)?;
            }
        } else if src.is_dir() {
            if dest.exists() {
                if src.contains_opq_file() {
                    remove(&dest)
                        .with_context(|| format!("Failed to remove: '{}'", dest.display()))?;
                    copy_dir(&src, &dest)?;
                }
            } else {
                copy_dir(&src, &dest)?;
            }
        } else {
            //do nothing
        }
    }
    Ok(())
}

fn copy_file<T, U>(src: T, dest: U) -> Result<()>
where
    T: AsRef<Path>,
    U: AsRef<Path>,
{
    copy(&src, &dest).with_context(|| {
        format!(
            "Failed to copy: '{}' to '{}'",
            src.as_ref().display(),
            dest.as_ref().display()
        )
    })?;

    Ok(())
}

fn copy_symlink<T, U>(src: T, dest: U) -> Result<()>
where
    T: AsRef<Path>,
    U: AsRef<Path>,
{
    let original = std::fs::read_link(&src)
        .with_context(|| format!("Failed to read link: '{}'", src.as_ref().display()))?;

    symlink(&original, &dest).with_context(|| {
        format!(
            "Failed to copy symlink: '{}' to '{}'",
            src.as_ref().display(),
            dest.as_ref().display()
        )
    })?;

    Ok(())
}

fn copy_dir<T, U>(src: T, dest: U) -> Result<()>
where
    T: AsRef<Path>,
    U: AsRef<Path>,
{
    let permissions = std::fs::metadata(&src)
        .with_context(|| format!("Failed to read metadata: '{}'", src.as_ref().display()))?
        .permissions();

    std::fs::create_dir(&dest)
        .with_context(|| format!("Failed to create dir: '{}'", dest.as_ref().display()))?;
    std::fs::set_permissions(&dest, permissions)
        .with_context(|| format!("Failed to set permissions: '{}'", dest.as_ref().display()))?;

    Ok(())
}

pub struct ImageMerger {
    lower: PathBuf,
    uppers: Vec<PathBuf>,
}

impl ImageMerger {
    pub fn new<P: AsRef<Path>>(lower: P) -> Self {
        Self {
            lower: lower.as_ref().to_path_buf(),
            uppers: Vec::new(),
        }
    }

    pub fn add_layer<T>(mut self, upper: T) -> Self
    where
        T: AsRef<Path>,
    {
        self.uppers.push(upper.as_ref().to_path_buf());
        self
    }

    pub fn add_layers<T>(mut self, uppers: Vec<T>) -> Self
    where
        T: AsRef<Path>,
    {
        let mut uppers_pathbuf: Vec<PathBuf> = uppers
            .into_iter()
            .map(|upper| upper.as_ref().to_path_buf())
            .collect();
        self.uppers.append(&mut uppers_pathbuf);
        self
    }

    pub fn merge(self) -> Result<()> {
        for upper in self.uppers {
            copy_layer(upper, &self.lower)?;
        }
        Ok(())
    }
}
