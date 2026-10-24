//! Stub: the API of the in-memory name-constrained CA, so that its tests compile red.

use oxedyne_fe2o3_core::prelude::*;

use std::sync::Arc;

use rcgen::CertificateParams;
use tokio_rustls::rustls::{
    pki_types::CertificateDer,
    server::{
        ClientHello,
        ResolvesServerCert,
    },
    sign::CertifiedKey,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameRule {
    Exact(String),
    Suffix(String),
}

#[derive(Debug)]
pub struct Mint;

impl Mint {
    pub fn new(_names: &[NameRule], _ca_days: u32) -> Outcome<Self> {
        Err(err!("Mint is not yet built."; Unimplemented))
    }
    pub fn ca_params(_names: &[NameRule], _ca_days: u32) -> Outcome<CertificateParams> {
        Err(err!("Mint is not yet built."; Unimplemented))
    }
    pub fn ca_pem(&self) -> String { String::new() }
    pub fn ca_der(&self) -> CertificateDer<'static> { CertificateDer::from(Vec::new()) }
    pub fn expires_within(&self, _secs: u64) -> bool { true }
    pub fn leaf(&self, _name: &str) -> Outcome<Arc<CertifiedKey>> {
        Err(err!("Mint is not yet built."; Unimplemented))
    }
}

impl ResolvesServerCert for Mint {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> { None }
}
