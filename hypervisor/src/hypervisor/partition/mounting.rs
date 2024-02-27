use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use bytesize::ByteSize;
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
        let relative_target = self.target.strip_prefix("/").unwrap_or(&self.target);
        println!("{relative_target:?}");
        let target: &PathBuf = &base_dir.join(relative_target);
        let fstype = self.fstype.map(PathBuf::from);
        let data = self.data.map(PathBuf::from);

        if let Some(src) = &self.source {
            Self::exists(src)?;
        }

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

    fn exists<T: AsRef<Path>>(path: T) -> anyhow::Result<()> {
        if !path.as_ref().exists() {
            bail!(
                "source file/dir does not exist: {}",
                path.as_ref().display()
            )
        }
        Ok(())
    }

    pub fn tmpfs<T: AsRef<Path>>(target: T, size: ByteSize) -> Self {
        FileMounter {
            source: None,
            target: target.as_ref().to_path_buf(),
            fstype: Some("tmpfs".into()),
            flags: MsFlags::empty(),
            data: Some(format!("size={}", size.0)),
            is_dir: true,
        }
    }

    pub fn cgroup() -> Self {
        FileMounter {
            source: None,
            target: "/sys/fs/cgroup".into(),
            fstype: Some("cgroup2".into()),
            flags: MsFlags::empty(),
            data: None,
            is_dir: true,
        }
    }

    pub fn proc() -> Self {
        FileMounter {
            source: Some("/proc".into()),
            target: "/proc".into(),
            fstype: Some("proc".into()),
            flags: MsFlags::empty(),
            data: None,
            is_dir: true,
        }
    }

    pub fn bind_ro<T: AsRef<Path>, U: AsRef<Path>>(source: T, target: U) -> anyhow::Result<Self> {
        Self::exists(&source)?;

        Ok(FileMounter {
            source: Some(source.as_ref().to_path_buf()),
            target: target.as_ref().to_path_buf(),
            fstype: None,
            flags: MsFlags::MS_RDONLY | MsFlags::MS_BIND,
            data: None,
            is_dir: source.as_ref().is_dir(),
        })
    }

    pub fn bind_rw<T: AsRef<Path>, U: AsRef<Path>>(source: T, target: U) -> anyhow::Result<Self> {
        Self::exists(&source)?;

        Ok(FileMounter {
            source: Some(source.as_ref().to_path_buf()),
            target: target.as_ref().to_path_buf(),
            fstype: None,
            flags: MsFlags::MS_BIND,
            data: None,
            is_dir: source.as_ref().is_dir(),
        })
    }
}
