use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use nix::mount::{mount, MsFlags};

/// Information about the files that are to be mounted
#[derive(Debug)]
pub struct FileMounter {
    source: Option<PathBuf>,
    target: PathBuf,
    fstype: Option<String>,
    flags: MsFlags,
    data: Option<String>,
    // TODO: Find a way to get rid of this boolean
    is_dir: bool, // Use File::create or fs::create_dir_all
}

impl FileMounter {
    // Mount (and consume) a device
    pub fn mount(self, base_dir: &Path) -> anyhow::Result<()> {
        let target: &PathBuf = &base_dir.join(self.target);
        let fstype = self.fstype.map(PathBuf::from);
        let data = self.data.map(PathBuf::from);

        if self.is_dir {
            trace!("Creating directory {}", target.display());
            fs::create_dir_all(target).context("failed to create target directory")?;
        } else {
            let parent = target.parent()
                .expect("target to have at least one direct parent directory because it was based on the `base_dir` path");
            trace!("Creating directory {}", parent.display());
            fs::create_dir_all(parent)
                .context("failed to create parent directory of target file")?;

            trace!("Creating file {}", target.display());
            fs::File::create(target).context("failed to create target file")?;
        }

        mount::<PathBuf, PathBuf, PathBuf, PathBuf>(
            self.source.as_ref(),
            target,
            fstype.as_ref(),
            self.flags,
            data.as_ref(),
        )
        .context("failed to make `nix::mount()` call")
    }

    /// Creates a new `FileMounter` from a source path and a relative target
    /// path.
    pub fn from_paths(source: PathBuf, target: PathBuf) -> anyhow::Result<Self> {
        Self::new(Some(source), target, None, MsFlags::MS_BIND, None)
    }

    pub fn new(
        source: Option<PathBuf>,
        mut target: PathBuf,
        fstype: Option<String>,
        flags: MsFlags,
        data: Option<String>,
    ) -> anyhow::Result<Self> {
        if let Some(source) = source.as_ref() {
            if !source.exists() {
                bail!("source file/dir does not exist: {}", source.display())
            }
        }

        if target.is_absolute() {
            // Convert absolute paths into relative ones.
            // Otherwise we will receive a permission error.
            // TODO: Make this a function?
            target = target
                .strip_prefix("/")
                .expect("the target path to start with '/' because it is an absolute path")
                .to_path_buf();
            assert!(target.is_relative());
        }

        let is_dir = source.as_ref().map_or(true, |source| source.is_dir());

        Ok(Self {
            source,
            target,
            fstype,
            flags,
            data,
            is_dir,
        })
    }
}
