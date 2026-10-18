use std::os::unix::fs::symlink;

use super::*;

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
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
}

#[test]
fn what_others_could_open_or_that_isnt_what_it_should_be_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join("d");
    super::dir(&dir).unwrap();
    let file = dir.join("f");
    place(&file, b"x", false).unwrap();
    chmod(&file, 0o644);
    assert_eq!(check(&file, false), Err(Error::Open { path: file.clone(), dir: false }));
    chmod(&file, 0o600);
    chmod(&dir, 0o755);
    assert_eq!(super::dir(&dir), Err(Error::Open { path: dir.clone(), dir: true }));
    chmod(&dir, 0o700);
    let link = dir.join("link");
    symlink(&file, &link).unwrap();
    assert_eq!(read(&link), Err(Error::Kind { path: link, dir: false }));
    assert!(matches!(read(&dir.join("missing")), Err(Error::Io { .. })));
    assert!(!check(&dir.join("missing"), false).unwrap());
}
