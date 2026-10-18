use std::os::unix::fs::PermissionsExt as _;

use super::*;

const NOW: Millis = 1_792_022_400_000;
const HOUR: Millis = 60 * 60 * 1000;

fn signer() -> Signer {
    Signer::from_seed([3; 32])
}

#[test]
fn a_token_names_its_run_and_holds_until_it_expires() {
    let signer = signer();
    let token = signer.mint("r1", "t1", NOW, HOUR).unwrap();
    let claims = signer.verifier().verify(&token, NOW).unwrap();
    assert_eq!((claims.run.as_str(), claims.thread.as_str()), ("r1", "t1"));
    assert_eq!((claims.issued, claims.expires), (NOW, NOW + HOUR));
    assert_eq!(claims.id.len(), 32);
    assert_ne!(signer.mint("r1", "t1", NOW, HOUR).unwrap(), token, "each token has its own id");
    let verify = |at| signer.verifier().verify(&token, at);
    assert!(verify(NOW + HOUR - 1).is_ok());
    assert_eq!(verify(NOW + HOUR), Err(Error::Expired));
    assert!(verify(NOW - SKEW).is_ok());
    assert_eq!(verify(NOW - SKEW - 1), Err(Error::Expired));
}

#[test]
fn a_lifetime_is_capped_and_a_token_names_a_run_and_a_thread() {
    let signer = signer();
    assert!(signer.mint("r", "t", NOW, MAX_LIFE).is_ok());
    for life in [0, -1, MAX_LIFE + 1] {
        assert_eq!(signer.mint("r", "t", NOW, life), Err(Error::Life(life)));
    }
    assert_eq!(signer.mint("", "t", NOW, HOUR), Err(Error::Unnamed));
    assert_eq!(signer.mint("r", "", NOW, HOUR), Err(Error::Unnamed));
}

#[test]
fn signed_claims_that_say_more_or_other_than_claims_are_refused() {
    let signer = signer();
    let claims = r#"{"id":"i","run":"r","thread":"t","issued":0,"expires":9999999999999,"actor":"actor/user"}"#;
    for bytes in [claims.as_bytes(), b"not json".as_slice()] {
        let token = format!("v1.{}.{}", hex(bytes), hex(&signer.0.sign(bytes).to_bytes()));
        assert_eq!(signer.verifier().verify(&token, NOW), Err(Error::Malformed));
    }
}

#[test]
fn only_this_operators_untouched_token_of_a_known_version_passes() {
    let signer = signer();
    let verifier = signer.verifier();
    let token = signer.mint("r1", "t1", NOW, HOUR).unwrap();
    let (claims, signature) = token.strip_prefix("v1.").unwrap().split_once('.').unwrap();
    let other = Signer::from_seed([4; 32]).mint("r1", "t1", NOW, HOUR).unwrap();
    assert_eq!(verifier.verify(&other, NOW), Err(Error::Signature));
    // Claims rewritten to name another run, with the original signature.
    let forged = hex(&serde_json::to_vec(&Claims {
        run: "r2".into(),
        ..verifier.verify(&token, NOW).unwrap()
    })
    .unwrap());
    assert_eq!(verifier.verify(&format!("v1.{forged}.{signature}"), NOW), Err(Error::Signature));
    for unknown in
        [format!("v2.{claims}.{signature}"), format!("V1.{claims}.{signature}"), String::new()]
    {
        assert_eq!(verifier.verify(&unknown, NOW), Err(Error::Version), "{unknown}");
    }
    for malformed in [
        "v1".to_owned(),
        format!("v1.{claims}"),
        format!("v1.{claims}.{signature}.x"),
        format!("v1.{}.{signature}", claims.to_uppercase()),
        format!("v1.{claims}.{}", &signature[2..]),
        format!("v1.{claims}x.{signature}"),
    ] {
        assert_eq!(verifier.verify(&malformed, NOW), Err(Error::Malformed), "{malformed}");
    }
}

#[test]
fn the_key_is_made_once_and_kept_private() {
    let home = tempfile::tempdir().unwrap();
    let (first, path) = Signer::open(home.path()).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let (again, _) = Signer::open(home.path()).unwrap();
    assert_eq!(first.verifier(), again.verifier());
    std::fs::write(&path, "not a key\n").unwrap();
    assert!(matches!(Signer::open(home.path()), Err(Error::Key(_))));
}

#[test]
fn hex_round_trips_and_refuses_anything_else() {
    assert_eq!(hex(&[0x00, 0x9f, 0xff]), "009fff");
    assert_eq!(unhex("009fff"), Some(vec![0x00, 0x9f, 0xff]));
    for bad in ["0", "0g", "FF", " 00"] {
        assert_eq!(unhex(bad), None, "{bad}");
    }
}
