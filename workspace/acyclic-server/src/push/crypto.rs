//! Web Push's two pieces of cryptography: the message encrypted to the browser's key (RFC 8291, `aes128gcm` from
//! RFC 8188), and the server's signed claim to the push service (VAPID, RFC 8292).

use aes_gcm::aead::{Aead as _, KeyInit as _};
use aes_gcm::{Aes128Gcm, Nonce};
use base64ct::{Base64UrlUnpadded, Encoding as _};
use hkdf::Hkdf;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::{Signature, SigningKey};
use p256::{PublicKey, SecretKey};
use sha2::Sha256;

/// The record size written into the header: one record, since a notification is far smaller.
const RECORD: u32 = 4096;

/// Why a message couldn't be encrypted or signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);

/// A browser's subscription keys, as `PushSubscription.getKey` gives them, base64url-decoded.
pub struct Receiver<'a> {
    /// The browser's P-256 public key, uncompressed (65 bytes).
    pub public: &'a [u8],
    /// The browser's authentication secret (16 bytes).
    pub auth: &'a [u8],
}

/// Encrypts `plain` to `to` as one `aes128gcm` record, with the server's one-off key `sender` and `salt`. Both must be
/// fresh for each message; tests pass fixed ones to check against the RFC's example.
///
/// # Errors
///
/// The browser's key or secret is malformed.
pub fn encrypt(
    plain: &[u8],
    to: &Receiver<'_>,
    sender: &SecretKey,
    salt: &[u8; 16],
) -> Result<Vec<u8>, Error> {
    if to.auth.len() != 16 {
        return Err(Error("the subscription's auth secret isn't 16 bytes"));
    }
    let browser = PublicKey::from_sec1_bytes(to.public)
        .map_err(|_| Error("the subscription's key isn't a P-256 point"))?;
    let server = sender.public_key().to_sec1_bytes();
    let shared = sender.diffie_hellman(&browser);
    // RFC 8291 section 3.4: the input keying material, from the shared secret and both public keys.
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(to.public);
    info.extend_from_slice(&server);
    let mut ikm = [0_u8; 32];
    Hkdf::<Sha256>::new(Some(to.auth), shared.raw_secret_bytes())
        .expand(&info, &mut ikm)
        .map_err(|_| Error("HKDF refused the key length"))?;
    // RFC 8188 section 2.2: the content encryption key and nonce.
    let content = Hkdf::<Sha256>::new(Some(salt), &ikm);
    let mut key = [0_u8; 16];
    let mut nonce = [0_u8; 12];
    content
        .expand(b"Content-Encoding: aes128gcm\0", &mut key)
        .map_err(|_| Error("HKDF refused the key length"))?;
    content
        .expand(b"Content-Encoding: nonce\0", &mut nonce)
        .map_err(|_| Error("HKDF refused the nonce length"))?;
    // The last, and only, record ends with the delimiter 2 and no padding.
    let mut record = plain.to_vec();
    record.push(2);
    let sealed = Aes128Gcm::new_from_slice(&key)
        .map_err(|_| Error("AES refused the key"))?
        .encrypt(&Nonce::from(nonce), record.as_slice())
        .map_err(|_| Error("AES-GCM failed"))?;
    let mut body = Vec::with_capacity(16 + 4 + 1 + server.len() + sealed.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD.to_be_bytes());
    body.push(u8::try_from(server.len()).map_err(|_| Error("the server's key is too long"))?);
    body.extend_from_slice(&server);
    body.extend_from_slice(&sealed);
    Ok(body)
}

/// The value of `Authorization` for a push to a service at `audience` (the endpoint's origin), as the holder of
/// `key`, claiming `subject` if any, valid until `expires` (seconds since the epoch, at most a day ahead).
#[must_use]
pub fn vapid(key: &SigningKey, audience: &str, subject: Option<&str>, expires: u64) -> String {
    let header = Base64UrlUnpadded::encode_string(br#"{"typ":"JWT","alg":"ES256"}"#);
    let mut claims = serde_json::json!({"aud": audience, "exp": expires});
    if let Some(subject) = subject {
        claims["sub"] = subject.into();
    }
    let claims = Base64UrlUnpadded::encode_string(claims.to_string().as_bytes());
    let signed = format!("{header}.{claims}");
    let signature: Signature = key.sign(signed.as_bytes());
    let token = format!("{signed}.{}", Base64UrlUnpadded::encode_string(&signature.to_bytes()));
    let public = Base64UrlUnpadded::encode_string(&key.verifying_key().to_sec1_bytes());
    format!("vapid t={token}, k={public}")
}

#[cfg(test)]
pub(crate) mod tests {
    use aes_gcm::aead::{Aead as _, KeyInit as _};
    use aes_gcm::{Aes128Gcm, Nonce};
    use base64ct::{Base64UrlUnpadded, Encoding as _};
    use hkdf::Hkdf;
    use p256::ecdsa::signature::Verifier as _;
    use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
    use p256::{PublicKey, SecretKey};
    use sha2::Sha256;

    use super::{Receiver, encrypt, vapid};

    pub(crate) fn b64(text: &str) -> Vec<u8> {
        Base64UrlUnpadded::decode_vec(text).unwrap()
    }

    /// Decrypts one `aes128gcm` record as the browser holding `secret` and `auth` would: the push service stub's half.
    pub(crate) fn decrypt(body: &[u8], secret: &SecretKey, auth: &[u8]) -> Vec<u8> {
        let (salt, rest) = body.split_at(16);
        let id_len = usize::from(rest[4]);
        let (server, sealed) = rest[5..].split_at(id_len);
        let shared = secret.diffie_hellman(&PublicKey::from_sec1_bytes(server).unwrap());
        let mut info = b"WebPush: info\0".to_vec();
        info.extend_from_slice(&secret.public_key().to_sec1_bytes());
        info.extend_from_slice(server);
        let mut ikm = [0_u8; 32];
        Hkdf::<Sha256>::new(Some(auth), shared.raw_secret_bytes()).expand(&info, &mut ikm).unwrap();
        let content = Hkdf::<Sha256>::new(Some(salt), &ikm);
        let (mut key, mut nonce) = ([0_u8; 16], [0_u8; 12]);
        content.expand(b"Content-Encoding: aes128gcm\0", &mut key).unwrap();
        content.expand(b"Content-Encoding: nonce\0", &mut nonce).unwrap();
        let mut plain =
            Aes128Gcm::new_from_slice(&key).unwrap().decrypt(&Nonce::from(nonce), sealed).unwrap();
        assert_eq!(plain.pop(), Some(2), "the last record ends with the delimiter 2");
        plain
    }

    /// Checks a VAPID `Authorization` value: its key is `public`, its token's signature verifies, and its claims.
    pub(crate) fn claims(header: &str, public: &[u8]) -> serde_json::Value {
        let (token, key) = header.strip_prefix("vapid t=").unwrap().split_once(", k=").unwrap();
        assert_eq!(b64(key), public);
        let (signed, signature) = token.rsplit_once('.').unwrap();
        let signature = Signature::from_slice(&b64(signature)).unwrap();
        VerifyingKey::from_sec1_bytes(public)
            .unwrap()
            .verify(signed.as_bytes(), &signature)
            .unwrap();
        let (header, claims) = signed.split_once('.').unwrap();
        assert_eq!(b64(header), br#"{"typ":"JWT","alg":"ES256"}"#);
        serde_json::from_slice(&b64(claims)).unwrap()
    }

    #[test]
    fn encrypts_rfc_8291s_example_exactly() {
        // RFC 8291, appendix A.
        let plain = b64("V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24");
        let sender =
            SecretKey::from_slice(&b64("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw")).unwrap();
        let browser = b64(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        );
        let auth = b64("BTBZMqHH6r4Tts7J_aSIgg");
        let salt: [u8; 16] = b64("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let body =
            encrypt(&plain, &Receiver { public: &browser, auth: &auth }, &sender, &salt).unwrap();
        assert_eq!(
            Base64UrlUnpadded::encode_string(&body),
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN"
        );
        // And the browser's half reads it back.
        let browser_secret =
            SecretKey::from_slice(&b64("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94")).unwrap();
        assert_eq!(decrypt(&body, &browser_secret, &auth), plain);
    }

    #[test]
    fn signs_a_vapid_claim_the_service_can_check() {
        let key = SigningKey::from_slice(&[7_u8; 32]).unwrap();
        let header = vapid(&key, "https://push.example", Some("https://acyclic.example"), 1_000);
        let public = key.verifying_key().to_sec1_bytes();
        assert_eq!(
            claims(&header, &public),
            serde_json::json!({"aud": "https://push.example", "exp": 1000, "sub": "https://acyclic.example"})
        );
        assert_eq!(
            claims(&vapid(&key, "https://push.example", None, 5), &public)["sub"],
            serde_json::Value::Null
        );
    }
}
