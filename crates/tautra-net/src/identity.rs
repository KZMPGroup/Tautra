use std::path::Path;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use rcgen::{CertificateParams, DistinguishedName, DnType, KeyPair, PKCS_ECDSA_P256_SHA256};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use tautra_common::{dpapi, paths};
use uuid::Uuid;

use crate::fingerprint::Fingerprint;

const IDENTITY_FILE: &str = "identity.toml";
const KEY_FILE: &str = "identity.key";

#[derive(Serialize, Deserialize)]
struct IdentityFile {
    device_id: Uuid,
    certificate: String,
    created_unix_ms: u64,
}

pub struct DeviceIdentity {
    pub device_id: Uuid,
    pub cert: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
    pub fingerprint: Fingerprint,
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("gagal membuat sertifikat: {0}")]
    Generate(#[from] rcgen::Error),
    #[error("gagal mengenkripsi kunci: {0}")]
    Dpapi(#[from] dpapi::DpapiError),
    #[error("gagal menulis identitas: {0}")]
    Io(#[from] std::io::Error),
    #[error("gagal menyusun identitas: {0}")]
    Serialize(#[from] toml::ser::Error),
}

impl DeviceIdentity {
    pub fn load_or_create(dir: &Path) -> Result<(Self, Option<String>), IdentityError> {
        match Self::load(dir) {
            Ok(Some(identity)) => Ok((identity, None)),
            Ok(None) => {
                let identity = Self::generate()?;
                identity.save(dir)?;
                tracing::info!(device_id = %identity.device_id, "identitas perangkat baru dibuat");
                Ok((identity, None))
            }
            Err(reason) => {
                tracing::error!("identitas lama tidak bisa dipakai: {reason}");
                let identity = Self::generate()?;
                identity.save(dir)?;
                Ok((
                    identity,
                    Some(format!(
                        "Identitas perangkat lama tidak bisa dibuka ({reason}). Identitas baru dibuat; pair ulang perangkat lain."
                    )),
                ))
            }
        }
    }

    pub fn private_key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(self.key.clone_key())
    }

    fn load(dir: &Path) -> Result<Option<Self>, String> {
        let id_path = dir.join(IDENTITY_FILE);
        let key_path = dir.join(KEY_FILE);
        let text = match std::fs::read_to_string(&id_path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("membaca {}: {e}", id_path.display())),
        };
        let file: IdentityFile = toml::from_str(&text)
            .map_err(|e| format!("{IDENTITY_FILE} tidak valid: {}", e.message()))?;
        let cert_der = B64
            .decode(file.certificate.trim())
            .map_err(|e| format!("sertifikat tidak valid: {e}"))?;
        let sealed = std::fs::read(&key_path).map_err(|e| format!("membaca {KEY_FILE}: {e}"))?;
        let key_der = dpapi::unprotect(&sealed).map_err(|e| e.to_string())?;
        KeyPair::try_from(key_der.as_slice())
            .map_err(|e| format!("kunci privat tidak valid: {e}"))?;

        let fingerprint = Fingerprint::of(&cert_der);
        Ok(Some(Self {
            device_id: file.device_id,
            cert: CertificateDer::from(cert_der),
            key: PrivatePkcs8KeyDer::from(key_der),
            fingerprint,
        }))
    }

    fn generate() -> Result<Self, IdentityError> {
        let device_id = Uuid::new_v4();
        let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let mut params = CertificateParams::new(vec![format!("tautra-{}", device_id.simple())])?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, format!("Tautra {device_id}"));
        params.distinguished_name = dn;
        let cert = params.self_signed(&key_pair)?;
        let cert_der = cert.der().clone();
        Ok(Self {
            device_id,
            fingerprint: Fingerprint::of(&cert_der),
            cert: cert_der,
            key: PrivatePkcs8KeyDer::from(key_pair.serialize_der()),
        })
    }

    fn save(&self, dir: &Path) -> Result<(), IdentityError> {
        let file = IdentityFile {
            device_id: self.device_id,
            certificate: B64.encode(self.cert.as_ref()),
            created_unix_ms: tautra_common::time::unix_ms(),
        };
        let sealed = dpapi::protect(self.key.secret_pkcs8_der())?;
        paths::write_atomic(&dir.join(KEY_FILE), &sealed)?;
        paths::write_atomic(
            &dir.join(IDENTITY_FILE),
            toml::to_string_pretty(&file)?.as_bytes(),
        )?;
        Ok(())
    }
}
