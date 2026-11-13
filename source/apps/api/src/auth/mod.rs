//! Authentication primitives. HTTP handlers and the request principal come after the
//! compatibility gate (`compat_tests`) passes.

pub mod account;
pub mod audit;
pub mod auth_log;
pub mod login;
pub mod node_version;
pub mod oidc;
pub mod password;
pub mod passkey_ceremony;
pub mod passkey_routes;
pub mod pending_login;
pub mod principal;
pub mod rbac;
pub mod routes;
pub mod session;
pub mod stream_guard;
pub mod setup;
pub mod throttle;
pub mod tokens;
pub mod totp;

pub use routes::router;

#[cfg(test)]
mod compat_tests;

/// Lookup key for access and node tokens: SHA-256 of the UTF-8 token as lowercase hex
/// (`packages/node-api/src/token.ts` `hashNodeToken`, `apps/web/lib/access-token.ts`).
pub fn token_digest(token: &str) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn token_digest_is_lowercase_sha256_hex() {
        assert_eq!(
            super::token_digest("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}

pub mod passkey;
