use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

pub(crate) fn replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace_with(path, |file| file.write_all(bytes))
}

fn replace_with(
    path: &Path,
    write: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let destination = destination(path)?;
    let parent = destination_parent(&destination);
    let permissions = existing_permissions(&destination)?;
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        builder.permissions(fs::Permissions::from_mode(0o666))
    };
    let mut temporary = builder.tempfile_in(parent)?;
    if let Some(permissions) = permissions {
        temporary.as_file().set_permissions(permissions)?;
    }
    write(temporary.as_file_mut())?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(&destination).map_err(|err| err.error)?;
    Ok(())
}

fn destination_parent(destination: &Path) -> &Path {
    destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn existing_permissions(destination: &Path) -> io::Result<Option<fs::Permissions>> {
    Ok(match fs::metadata(destination) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => None,
        Err(err) => return Err(err),
    })
}

fn destination(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..40 {
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let target = fs::read_link(&current)?;
                current = if target.is_absolute() {
                    target
                } else {
                    current
                        .parent()
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .unwrap_or_else(|| Path::new("."))
                        .join(target)
                };
            }
            Ok(_) => return Ok(current),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "symbolic link chain is too deep",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_write_preserves_the_original() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("source.txt");
        fs::write(&path, b"original").expect("seed original");

        let err = replace_with(&path, |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("injected write failure"))
        })
        .expect_err("injected failure must surface");

        assert_eq!(err.to_string(), "injected write failure");
        assert_eq!(fs::read(&path).expect("read original"), b"original");
    }

    #[test]
    fn replacement_preserves_permissions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("source.txt");
        fs::write(&path, b"original").expect("seed original");
        let permissions = fs::metadata(&path).expect("metadata").permissions();

        replace(&path, b"replacement").expect("replace");

        assert_eq!(fs::read(&path).expect("read replacement"), b"replacement");
        assert_eq!(
            fs::metadata(&path).expect("metadata").permissions(),
            permissions
        );
    }

    #[test]
    fn replacement_creates_a_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("source.txt");

        replace(&path, b"new").expect("replace");

        assert_eq!(fs::read(&path).expect("read new file"), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn missing_file_uses_the_process_default_creation_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let reference = dir.path().join("reference.txt");
        let replacement = dir.path().join("replacement.txt");
        fs::File::create(&reference).expect("create reference");

        replace(&replacement, b"replacement").expect("replace");

        let reference_mode = fs::metadata(reference)
            .expect("reference metadata")
            .permissions()
            .mode();
        let replacement_mode = fs::metadata(replacement)
            .expect("replacement metadata")
            .permissions()
            .mode();
        assert_eq!(replacement_mode & 0o777, reference_mode & 0o777);
    }

    #[cfg(unix)]
    #[test]
    fn replacement_follows_an_existing_symlink() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        fs::write(&target, b"original").expect("seed target");
        symlink(&target, &link).expect("create symlink");

        replace(&link, b"replacement").expect("replace target");

        assert!(
            fs::symlink_metadata(&link)
                .expect("link metadata")
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&target).expect("read target"), b"replacement");
    }

    #[cfg(unix)]
    #[test]
    fn replacement_follows_a_dangling_relative_symlink() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        symlink("target.txt", &link).expect("create symlink");

        replace(&link, b"replacement").expect("create target");

        assert!(
            fs::symlink_metadata(&link)
                .expect("link metadata")
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(&target).expect("read target"), b"replacement");
    }

    #[cfg(unix)]
    #[test]
    fn destination_propagates_non_missing_metadata_errors() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;

        let path = Path::new(OsStr::from_bytes(b"invalid\0path"));
        let error = destination(path).expect_err("an invalid path is not a missing file");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn bare_destination_uses_the_current_directory() {
        assert_eq!(destination_parent(Path::new("source.txt")), Path::new("."));
    }

    #[cfg(unix)]
    #[test]
    fn permission_probe_propagates_non_missing_metadata_errors() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;

        let path = Path::new(OsStr::from_bytes(b"invalid\0path"));
        let error = existing_permissions(path).expect_err("invalid metadata path");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
