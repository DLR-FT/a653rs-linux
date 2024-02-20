use std::fs;
use std::path::{Path, PathBuf};

use anyhow::bail;
use nix::mount::{mount, MsFlags};

/// Information about the files that are to be mounted
#[derive(Debug)]
pub struct FileMounter {
    pub source: Option<PathBuf>,
    pub target: PathBuf,
    pub fstype: Option<String>,
    pub flags: MsFlags,
    pub data: Option<String>,
    // TODO: Find a way to get rid of this boolean
    pub is_dir: bool, // Use File::create or fs::create_dir_all
}

impl FileMounter {
    // Mount (and consume) a device
    pub fn mount(self, base_dir: &Path) -> anyhow::Result<()> {
        let target: &PathBuf = &base_dir.join(self.target);
        let fstype = self.fstype.map(PathBuf::from);
        let data = self.data.map(PathBuf::from);

        if self.is_dir {
            trace!("Creating directory {}", target.display());
            fs::create_dir_all(target)?;
        } else {
            // It is okay to use .unwrap() here.
            // It will only fail due to a developer mistake, not due to a user mistake.
            let parent = target.parent().unwrap();
            trace!("Creating directory {}", parent.display());
            fs::create_dir_all(parent)?;

            trace!("Creating file {}", target.display());
            fs::File::create(target)?;
        }

        mount::<PathBuf, PathBuf, PathBuf, PathBuf>(
            self.source.as_ref(),
            target,
            fstype.as_ref(),
            self.flags,
            data.as_ref(),
        )?;

        anyhow::Ok(())
    }
}

impl TryFrom<&(PathBuf, PathBuf)> for FileMounter {
    type Error = anyhow::Error;

    fn try_from(paths: &(PathBuf, PathBuf)) -> Result<Self, Self::Error> {
        let source = &paths.0;
        let mut target = paths.1.clone();

        if !source.exists() {
            bail!("File/Directory {} not existent", source.display())
        }

        if target.is_absolute() {
            // Convert absolute paths into relative ones.
            // Otherwise we will receive a permission error.
            // TODO: Make this a function?
            target = target.strip_prefix("/")?.into();
            assert!(target.is_relative());
        }

        Ok(Self {
            source: Some(source.clone()),
            target,
            fstype: None,
            flags: MsFlags::MS_BIND,
            data: None,
            is_dir: source.is_dir(),
        })
    }
}
