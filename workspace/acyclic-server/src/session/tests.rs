use std::os::unix::fs::symlink;

use super::*;

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn the_session_is_made_private_once_and_kept_until_rotated() {
    let home = tempfile::tempdir().unwrap();
    let (token, path) = open(home.path(), false).unwrap();
    assert_eq!(path, home.path().join(".config/acyclic/up/session"));
    assert_eq!((mode(path.parent().unwrap()), mode(&path)), (0o700, 0o600));
    assert_eq!(open(home.path(), false).unwrap().0, token);
    let (rotated, _) = open(home.path(), true).unwrap();
    assert_ne!(rotated, token);
    assert_eq!(open(home.path(), false).unwrap().0, rotated);
    assert_eq!(mode(&path), 0o600);
    // Nothing staged is left behind.
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn a_session_others_could_read_or_that_holds_no_token_is_refused() {
    let refused = |prepare: &dyn Fn(&Path)| {
        let home = tempfile::tempdir().unwrap();
        let (_, path) = open(home.path(), false).unwrap();
        prepare(&path);
        open(home.path(), false).unwrap_err().code
    };
    let chmod =
        |path: &Path, mode| fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    assert_eq!(refused(&|path| chmod(path, 0o644)), "session");
    assert_eq!(refused(&|path| chmod(path.parent().unwrap(), 0o755)), "session");
    assert_eq!(refused(&|path| fs::write(path, "").unwrap()), "session");
    assert_eq!(refused(&|path| fs::write(path, "not a token\n").unwrap()), "session");
    assert_eq!(
        refused(&|path| {
            let elsewhere = path.with_file_name("elsewhere");
            fs::rename(path, &elsewhere).unwrap();
            symlink(&elsewhere, path).unwrap();
        }),
        "session"
    );
}

const TEXT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn a_token_is_exactly_its_hex_digits() {
    assert!(Token::parse(TEXT).is_some());
    assert!(Token::parse(&format!("{TEXT}\n")).is_some());
    for bad in
        ["", "\n", &TEXT[1..], &TEXT.to_uppercase(), &format!("{TEXT}\n\n"), &format!(" {TEXT}")]
    {
        assert!(Token::parse(bad).is_none(), "{bad:?}");
    }
}

#[test]
fn comparing_takes_every_byte_into_account() {
    assert!(equal(b"abcd", b"abcd"));
    assert!(!equal(b"abcd", b"abce"));
    assert!(!equal(b"abcd", b"bbcd"));
    assert!(!equal(b"abcd", b"abc"));
    assert!(!equal(b"", b"a"));
}
