use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{IdleTimeout, TransportConfig, VarInt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{CertificateError, DigitallySignedStruct, DistinguishedName, SignatureScheme};
use tautra_proto::{ALPN_PAIRING, ALPN_PROBE, ALPN_SESSION, ALPN_UPDATE};

use crate::fingerprint::Fingerprint;
use crate::identity::DeviceIdentity;

pub const SERVER_NAME: &str = "tautra";

pub const KEEP_ALIVE: Duration = Duration::from_secs(1);
pub const IDLE_TIMEOUT_MS: u32 = 3_000;

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("konfigurasi TLS gagal: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("konfigurasi QUIC gagal: {0}")]
    Quic(#[from] quinn::crypto::rustls::NoInitialCipherSuite),
}

pub struct TlsContext {
    provider: Arc<CryptoProvider>,
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    transport: Arc<TransportConfig>,
}

impl TlsContext {
    pub fn new(identity: &DeviceIdentity) -> Self {
        Self {
            provider: Arc::new(rustls::crypto::ring::default_provider()),
            cert: identity.cert.clone(),
            key: identity.private_key(),
            transport: Arc::new(transport_config()),
        }
    }

    pub fn server_config(&self) -> Result<quinn::ServerConfig, TlsError> {
        let verifier = Arc::new(AnyClientCert {
            provider: self.provider.clone(),
        });
        let mut crypto = rustls::ServerConfig::builder_with_provider(self.provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .with_client_cert_verifier(verifier)
            .with_single_cert(vec![self.cert.clone()], self.key.clone_key())?;
        crypto.alpn_protocols = vec![
            ALPN_SESSION.to_vec(),
            ALPN_PAIRING.to_vec(),
            ALPN_PROBE.to_vec(),
            ALPN_UPDATE.to_vec(),
        ];
        let mut config =
            quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(crypto)?));
        config.transport_config(self.transport.clone());
        Ok(config)
    }

    pub fn client_config(
        &self,
        alpn: &'static [u8],
        expected: Option<Fingerprint>,
    ) -> Result<(quinn::ClientConfig, Arc<AtomicBool>), TlsError> {
        let mismatch = Arc::new(AtomicBool::new(false));
        let verifier = Arc::new(PinnedServerCert {
            provider: self.provider.clone(),
            expected,
            mismatch: mismatch.clone(),
        });
        let mut crypto = rustls::ClientConfig::builder_with_provider(self.provider.clone())
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_client_auth_cert(vec![self.cert.clone()], self.key.clone_key())?;
        crypto.alpn_protocols = vec![alpn.to_vec()];
        let mut config = quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(crypto)?));
        config.transport_config(self.transport.clone());
        Ok((config, mismatch))
    }
}

fn transport_config() -> TransportConfig {
    let mut t = TransportConfig::default();
    t.max_idle_timeout(Some(IdleTimeout::from(VarInt::from_u32(IDLE_TIMEOUT_MS))));
    t.keep_alive_interval(Some(KEEP_ALIVE));
    t.datagram_receive_buffer_size(Some(8 * 1024 * 1024));
    t.datagram_send_buffer_size(4 * 1024 * 1024);
    t
}

pub fn peer_fingerprint(conn: &quinn::Connection) -> Option<Fingerprint> {
    let identity = conn.peer_identity()?;
    let certs = identity.downcast::<Vec<CertificateDer<'static>>>().ok()?;
    certs.first().map(|c| Fingerprint::of(c))
}

pub fn negotiated_alpn(conn: &quinn::Connection) -> Option<Vec<u8>> {
    let data = conn
        .handshake_data()?
        .downcast::<quinn::crypto::rustls::HandshakeData>()
        .ok()?;
    data.protocol
}

#[derive(Debug)]
struct AnyClientCert {
    provider: Arc<CryptoProvider>,
}

impl ClientCertVerifier for AnyClientCert {
    fn client_auth_mandatory(&self) -> bool {
        true
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[derive(Debug)]
struct PinnedServerCert {
    provider: Arc<CryptoProvider>,
    expected: Option<Fingerprint>,
    mismatch: Arc<AtomicBool>,
}

impl ServerCertVerifier for PinnedServerCert {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.expected {
            Some(expected) if Fingerprint::of(end_entity) != expected => {
                self.mismatch.store(true, Ordering::Relaxed);
                Err(rustls::Error::InvalidCertificate(
                    CertificateError::ApplicationVerificationFailure,
                ))
            }
            _ => Ok(ServerCertVerified::assertion()),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
