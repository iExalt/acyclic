use std::os::unix::fs::PermissionsExt as _;

use super::*;

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn a_private_directory_and_file_are_made_closed_and_kept() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("a/b");
    super::dir(&dir).unwrap();
    super::dir(&dir).unwrap();
    assert_eq!(mode(&dir), 0o700);
    let file = dir.join("f");
    assert!(place(&file, b"one", false).unwrap());
    assert_eq!(mode(&file), 0o600);
    // Another process got there first: its file stays.
    assert!(!place(&file, b"two", false).unwrap());
    assert_eq!(read(&file).unwrap(), "one");
    assert!(place(&file, b"three", true).unwrap());
    assert_eq!(read(&file).unwrap(), "three");
    // Nothing staged is left behind.
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    assert!(read(&dir.join("missing")).is_err());
}
