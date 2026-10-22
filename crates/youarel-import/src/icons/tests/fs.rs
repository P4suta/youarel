use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

pub(super) struct Unreadable {
    path: PathBuf,
    permissions: std::fs::Permissions,
}

impl Unreadable {
    pub(super) fn new(path: &Path) -> Self {
        let permissions = std::fs::metadata(path).unwrap().permissions();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
        Self {
            path: path.to_owned(),
            permissions,
        }
    }
}

impl Drop for Unreadable {
    fn drop(&mut self) {
        if let Err(error) = std::fs::set_permissions(&self.path, self.permissions.clone()) {
            eprintln!("cannot restore fixture permissions: {error}");
        }
    }
}
