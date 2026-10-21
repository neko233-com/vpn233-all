//! HS256 adapter for jsonwebtoken's public CryptoProvider API.
//!
//! JWT parsing, claims and algorithm validation remain in jsonwebtoken. Only the
//! required HMAC-SHA256 primitive is enabled, avoiding unused RSA/EC backends.

use std::sync::Once;

use hmac::{Hmac, Mac};
use jsonwebtoken::crypto::{CryptoProvider, JwkUtils, JwtSigner, JwtVerifier};
use jsonwebtoken::errors::{ErrorKind, Result};
use jsonwebtoken::signature::{self, Signer, Verifier};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey};
use sha2::Sha256;

struct Hs256(Hmac<Sha256>);

impl Hs256 {
    fn new(secret: &[u8]) -> Result<Self> {
        Hmac::<Sha256>::new_from_slice(secret)
            .map(Self)
            .map_err(|_| ErrorKind::InvalidKeyFormat.into())
    }
}

impl Signer<Vec<u8>> for Hs256 {
    fn try_sign(&self, message: &[u8]) -> std::result::Result<Vec<u8>, signature::Error> {
        let mut mac = self.0.clone();
        mac.update(message);
        Ok(mac.finalize().into_bytes().to_vec())
    }
}

impl Verifier<Vec<u8>> for Hs256 {
    fn verify(&self, message: &[u8], tag: &Vec<u8>) -> std::result::Result<(), signature::Error> {
        let mut mac = self.0.clone();
        mac.update(message);
        mac.verify_slice(tag).map_err(|_| signature::Error::new())
    }
}

impl JwtSigner for Hs256 {
    fn algorithm(&self) -> Algorithm {
        Algorithm::HS256
    }
}

impl JwtVerifier for Hs256 {
    fn algorithm(&self) -> Algorithm {
        Algorithm::HS256
    }
}

fn signer(algorithm: &Algorithm, key: &EncodingKey) -> Result<Box<dyn JwtSigner>> {
    if *algorithm != Algorithm::HS256 {
        return Err(ErrorKind::InvalidAlgorithm.into());
    }
    Ok(Box::new(Hs256::new(key.try_get_hmac_secret()?)?))
}

fn verifier(algorithm: &Algorithm, key: &DecodingKey) -> Result<Box<dyn JwtVerifier>> {
    if *algorithm != Algorithm::HS256 {
        return Err(ErrorKind::InvalidAlgorithm.into());
    }
    Ok(Box::new(Hs256::new(key.try_get_hmac_secret()?)?))
}

static PROVIDER: CryptoProvider = CryptoProvider {
    signer_factory: signer,
    verifier_factory: verifier,
    // This service does not import public keys or JWKs.
    jwk_utils: JwkUtils::new_unimplemented(),
};

pub(crate) fn install() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        PROVIDER
            .install_default()
            .expect("JWT crypto provider must be installed before use");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_hmac_vector_and_tampered_signatures() {
        let mac = Hs256::new(b"key").unwrap();
        let message = b"The quick brown fox jumps over the lazy dog";
        let signature = mac.try_sign(message).unwrap();
        let hex: String = signature.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(
            hex,
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
        assert!(mac.verify(message, &signature).is_ok());
        assert!(mac.verify(b"altered message", &signature).is_err());
        assert!(mac.verify(message, &signature[..31].to_vec()).is_err());
        assert!(Hs256::new(b"another key")
            .unwrap()
            .verify(message, &signature)
            .is_err());
    }

    #[test]
    fn factories_reject_every_other_algorithm() {
        for algorithm in [
            Algorithm::HS384,
            Algorithm::HS512,
            Algorithm::RS256,
            Algorithm::PS256,
            Algorithm::ES256,
            Algorithm::EdDSA,
        ] {
            assert!(signer(&algorithm, &EncodingKey::from_secret(b"key")).is_err());
            assert!(verifier(&algorithm, &DecodingKey::from_secret(b"key")).is_err());
        }
    }
}
