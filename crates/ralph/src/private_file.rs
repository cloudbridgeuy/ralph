use std::fs::{self, OpenOptions};
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

pub(crate) fn check_path(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::other(format!(
            "path is a symlink: {}",
            path.display()
        ))),
        #[cfg(unix)]
        Ok(metadata) if metadata.is_file() && metadata.nlink() > 1 => Err(io::Error::other(
            format!("file has multiple links: {}", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn open_private(path: &Path) -> io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    options.open(path)
}

pub(crate) fn write_private(path: PathBuf, bytes: &[u8]) -> io::Result<PathBuf> {
    check_path(&path)?;
    let mut file = open_private(&path)?;
    file.write_all(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    #[test]
    fn the_open_step_refuses_a_symlink_and_leaves_its_target_alone() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("target");
        fs::write(&target, "keep").unwrap();
        let link = dir.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(open_private(&link).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
    }

    #[test]
    fn a_private_write_creates_a_0600_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = write_private(dir.path().join("f"), b"x").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"x");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
