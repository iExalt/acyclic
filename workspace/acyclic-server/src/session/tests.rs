use std::fs;
use std::os::unix::fs::PermissionsExt as _;

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
fn only_a_session_already_made_exists() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(existing(home.path()).unwrap_err().code, "no-session");
    assert!(!home.path().join(DIR).join("session").exists());
    let (token, _) = open(home.path(), false).unwrap();
    assert_eq!(existing(home.path()).unwrap().0, token);
}

#[test]
fn a_session_file_that_holds_no_token_is_refused() {
    for text in ["", "not a token\n"] {
        let home = tempfile::tempdir().unwrap();
        let (_, path) = open(home.path(), false).unwrap();
        std::fs::write(&path, text).unwrap();
        assert_eq!(open(home.path(), false).unwrap_err().code, "session", "{text:?}");
    }
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

#[test]
fn hmac_matches_rfc_4231() {
    assert_eq!(
        hex(&hmac(b"Jefe", b"what do ya want for nothing?")),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
    let key = [0xaa; 131];
    assert_eq!(
        hex(&hmac(&key, b"Test Using Larger Than Block-Size Key - Hash Key First")),
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
    );
}

#[test]
fn a_code_is_good_only_for_its_session_and_its_few_minutes() {
    let token = Token::parse(TEXT).unwrap();
    let other = Token::parse(&TEXT.replace('0', "1")).unwrap();
    let now = 1_792_022_400_000;
    let code = token.code(now, [9; 8]);
    assert_eq!(token.verify(&code, now), Some(now));
    assert_eq!(token.verify(&code, now + CODE_LIFE), Some(now));
    assert_eq!(token.verify(&code, now + CODE_LIFE + 1), None);
    assert_eq!(token.verify(&code, now - CODE_SKEW), Some(now));
    assert_eq!(token.verify(&code, now - CODE_SKEW - 1), None);
    assert_eq!(other.verify(&code, now), None);
    let (stamped, mac) = code.rsplit_once('.').unwrap();
    let (made, nonce) = stamped.split_once('.').unwrap();
    let later = (now + 1).to_string();
    for forged in [
        format!("{later}.{nonce}.{mac}"),
        format!("{made}.{}.{mac}", "1".repeat(16)),
        format!("{made}.{nonce}.{}", "0".repeat(64)),
        format!("{made}.{nonce}"),
        format!("{made}.{}.{mac}", &nonce[2..]),
        String::new(),
    ] {
        assert_eq!(token.verify(&forged, now), None, "{forged}");
    }
    assert_ne!(token.browser(), other.browser());
}

#[test]
fn a_code_is_spent_once_and_forgotten_once_expired() {
    let spent = Spent::default();
    assert!(spent.spend("a", 0, 0));
    assert!(!spent.spend("a", 0, CODE_LIFE));
    assert!(spent.spend("b", CODE_LIFE, CODE_LIFE + 1));
    assert_eq!(spent.0.lock().unwrap().len(), 1);
}

#[test]
fn the_tailscale_secret_is_128_bits_of_its_own_session() {
    let one = Token::parse(&"1".repeat(64)).unwrap();
    let two = Token::parse(&"2".repeat(64)).unwrap();
    assert_eq!(one.tailscale().len(), 32);
    assert!(one.tailscale().bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(one.tailscale(), one.tailscale());
    assert_ne!(one.tailscale(), two.tailscale());
    assert_ne!(one.tailscale(), one.browser()[..32]);
}
