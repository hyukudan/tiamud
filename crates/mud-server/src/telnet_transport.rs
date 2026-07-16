use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use std::{
    env,
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
    sync::{OwnedSemaphorePermit, Semaphore},
    time::timeout,
};
use tokio_rustls::TlsAcceptor;

const DEFAULT_HANDSHAKE_TIMEOUT_SECONDS: u64 = 10;
const MAX_HANDSHAKE_TIMEOUT_SECONDS: u64 = 60;
const DEFAULT_MAX_PENDING_HANDSHAKES: usize = 128;
const MAX_PENDING_HANDSHAKES: usize = 4_096;

pub(crate) trait TelnetIo: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T> TelnetIo for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub(crate) type TelnetStream = Box<dyn TelnetIo>;

#[derive(Clone)]
pub(crate) struct TelnetTransport {
    mode: TransportMode,
    pending_handshakes: Arc<Semaphore>,
    max_pending_handshakes: usize,
}

#[derive(Clone)]
enum TransportMode {
    Tls {
        acceptor: TlsAcceptor,
        handshake_timeout: Duration,
    },
    Plaintext,
}

impl TelnetTransport {
    pub(crate) fn from_environment() -> io::Result<Self> {
        let mode = optional_env("TIAMUD_TELNET_MODE")?.unwrap_or_else(|| OsString::from("tls"));
        let certificate = optional_env("TIAMUD_TELNET_TLS_CERT")?.map(PathBuf::from);
        let private_key = optional_env("TIAMUD_TELNET_TLS_KEY")?.map(PathBuf::from);
        let handshake_timeout = optional_env("TIAMUD_TELNET_TLS_HANDSHAKE_TIMEOUT_SECONDS")?;
        let max_pending = optional_env("TIAMUD_TELNET_MAX_PENDING_HANDSHAKES")?;

        Self::from_values(
            &mode.to_string_lossy(),
            certificate.as_deref(),
            private_key.as_deref(),
            handshake_timeout.as_deref(),
            max_pending.as_deref(),
        )
    }

    fn from_values(
        mode: &str,
        certificate: Option<&Path>,
        private_key: Option<&Path>,
        handshake_timeout: Option<&std::ffi::OsStr>,
        max_pending: Option<&std::ffi::OsStr>,
    ) -> io::Result<Self> {
        let max_pending_handshakes = parse_bounded_usize(
            "TIAMUD_TELNET_MAX_PENDING_HANDSHAKES",
            max_pending,
            DEFAULT_MAX_PENDING_HANDSHAKES,
            1,
            MAX_PENDING_HANDSHAKES,
        )?;
        let pending_handshakes = Arc::new(Semaphore::new(max_pending_handshakes));

        match mode.trim().to_ascii_lowercase().as_str() {
            "tls" => {
                let certificate = certificate.ok_or_else(|| {
                    invalid_config("TIAMUD_TELNET_MODE=tls requires TIAMUD_TELNET_TLS_CERT")
                })?;
                let private_key = private_key.ok_or_else(|| {
                    invalid_config("TIAMUD_TELNET_MODE=tls requires TIAMUD_TELNET_TLS_KEY")
                })?;
                let seconds = parse_bounded_usize(
                    "TIAMUD_TELNET_TLS_HANDSHAKE_TIMEOUT_SECONDS",
                    handshake_timeout,
                    DEFAULT_HANDSHAKE_TIMEOUT_SECONDS as usize,
                    1,
                    MAX_HANDSHAKE_TIMEOUT_SECONDS as usize,
                )?;
                let acceptor = load_tls_acceptor(certificate, private_key)?;
                Ok(Self {
                    mode: TransportMode::Tls {
                        acceptor,
                        handshake_timeout: Duration::from_secs(seconds as u64),
                    },
                    pending_handshakes,
                    max_pending_handshakes,
                })
            }
            "plaintext" => Ok(Self {
                mode: TransportMode::Plaintext,
                pending_handshakes,
                max_pending_handshakes,
            }),
            _ => Err(invalid_config(
                "TIAMUD_TELNET_MODE must be exactly tls or plaintext",
            )),
        }
    }

    pub(crate) const fn is_tls(&self) -> bool {
        matches!(self.mode, TransportMode::Tls { .. })
    }

    pub(crate) const fn mode_name(&self) -> &'static str {
        if self.is_tls() { "tls" } else { "plaintext" }
    }

    pub(crate) const fn max_pending_handshakes(&self) -> usize {
        self.max_pending_handshakes
    }

    pub(crate) async fn reserve_handshake(&self) -> io::Result<OwnedSemaphorePermit> {
        self.pending_handshakes
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| io::Error::other("Telnet listener handshake limiter closed"))
    }

    pub(crate) async fn upgrade(&self, stream: TcpStream) -> io::Result<TelnetStream> {
        match &self.mode {
            TransportMode::Plaintext => Ok(Box::new(stream)),
            TransportMode::Tls {
                acceptor,
                handshake_timeout,
            } => {
                let stream = timeout(*handshake_timeout, acceptor.accept(stream))
                    .await
                    .map_err(|_| {
                        io::Error::new(io::ErrorKind::TimedOut, "Telnet TLS handshake timed out")
                    })?
                    .map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Telnet TLS handshake failed: {error}"),
                        )
                    })?;
                Ok(Box::new(stream))
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn plaintext_for_tests() -> Self {
        Self::from_values("plaintext", None, None, None, None)
            .expect("the fixed plaintext test transport is valid")
    }
}

fn optional_env(name: &str) -> io::Result<Option<OsString>> {
    match env::var_os(name) {
        Some(value) if value.is_empty() => Err(invalid_config(format!("{name} must not be empty"))),
        value => Ok(value),
    }
}

fn parse_bounded_usize(
    name: &str,
    value: Option<&std::ffi::OsStr>,
    default: usize,
    minimum: usize,
    maximum: usize,
) -> io::Result<usize> {
    let Some(value) = value else {
        return Ok(default);
    };
    let text = value
        .to_str()
        .ok_or_else(|| invalid_config(format!("{name} must be valid UTF-8")))?;
    let parsed = text
        .parse::<usize>()
        .map_err(|error| invalid_config(format!("invalid {name}={text:?}: {error}")))?;
    if !(minimum..=maximum).contains(&parsed) {
        return Err(invalid_config(format!(
            "{name} must be between {minimum} and {maximum}, got {parsed}"
        )));
    }
    Ok(parsed)
}

fn load_tls_acceptor(certificate_path: &Path, private_key_path: &Path) -> io::Result<TlsAcceptor> {
    let certificates = CertificateDer::pem_file_iter(certificate_path)
        .map_err(|error| {
            invalid_config(format!(
                "could not open Telnet TLS certificate {}: {error}",
                certificate_path.display()
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            invalid_config(format!(
                "could not parse Telnet TLS certificate {}: {error}",
                certificate_path.display()
            ))
        })?;
    if certificates.is_empty() {
        return Err(invalid_config(format!(
            "Telnet TLS certificate {} contains no certificates",
            certificate_path.display()
        )));
    }

    let private_key = PrivateKeyDer::from_pem_file(private_key_path).map_err(|error| {
        invalid_config(format!(
            "could not parse Telnet TLS private key {}: {error}",
            private_key_path.display()
        ))
    })?;

    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
            .map_err(|error| {
                invalid_config(format!("could not enable safe TLS versions: {error}"))
            })?
            .with_no_client_auth()
            .with_single_cert(certificates, private_key)
            .map_err(|error| invalid_config(format!("invalid Telnet TLS identity: {error}")))?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

fn invalid_config(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
    use std::fs;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::TlsConnector;

    #[test]
    fn tls_is_the_only_implicit_mode_and_requires_an_identity() {
        let error = TelnetTransport::from_values("tls", None, None, None, None)
            .err()
            .expect("missing TLS identity must fail");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("TIAMUD_TELNET_TLS_CERT"));
    }

    #[test]
    fn plaintext_requires_an_explicit_exact_mode() {
        let transport = TelnetTransport::from_values("plaintext", None, None, None, None).unwrap();
        assert!(!transport.is_tls());
        assert_eq!(transport.mode_name(), "plaintext");

        let transport = TelnetTransport::from_values(
            "plaintext",
            Some(Path::new("cert.pem")),
            None,
            None,
            None,
        )
        .expect("an explicit plaintext mode ignores dormant TLS identity variables");
        assert!(!transport.is_tls());

        let error = TelnetTransport::from_values("yes", None, None, None, None)
            .err()
            .expect("implicit plaintext aliases must fail");
        assert!(error.to_string().contains("exactly tls or plaintext"));
    }

    #[test]
    fn transport_limits_are_strictly_bounded() {
        let error = TelnetTransport::from_values(
            "plaintext",
            None,
            None,
            None,
            Some(std::ffi::OsStr::new("0")),
        )
        .err()
        .expect("zero pending handshakes must fail");
        assert!(error.to_string().contains("between 1 and 4096"));

        let error = parse_bounded_usize(
            "TIAMUD_TELNET_TLS_HANDSHAKE_TIMEOUT_SECONDS",
            Some(std::ffi::OsStr::new("61")),
            10,
            1,
            60,
        )
        .expect_err("excessive handshake timeout must fail");
        assert!(error.to_string().contains("between 1 and 60"));
    }

    #[tokio::test]
    async fn configured_tls_transport_completes_a_real_handshake() {
        let rcgen::CertifiedKey { cert, key_pair } =
            rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let certificate_path = directory.path().join("fullchain.pem");
        let private_key_path = directory.path().join("private-key.pem");
        fs::write(&certificate_path, cert.pem()).unwrap();
        fs::write(&private_key_path, key_pair.serialize_pem()).unwrap();

        let transport = TelnetTransport::from_values(
            "tls",
            Some(&certificate_path),
            Some(&private_key_path),
            Some(std::ffi::OsStr::new("2")),
            Some(std::ffi::OsStr::new("1")),
        )
        .unwrap();
        assert!(transport.is_tls());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let permit = transport.reserve_handshake().await.unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = transport.upgrade(stream).await.unwrap();
            drop(permit);
            let mut request = [0_u8; 4];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").await.unwrap();
        });

        let mut roots = RootCertStore::empty();
        roots.add(cert.der().clone()).unwrap();
        let client_config =
            ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
                .unwrap()
                .with_root_certificates(roots)
                .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(client_config));
        let stream = TcpStream::connect(address).await.unwrap();
        let server_name = ServerName::try_from("localhost").unwrap().to_owned();
        let mut stream = connector.connect(server_name, stream).await.unwrap();
        stream.write_all(b"ping").await.unwrap();
        let mut response = [0_u8; 4];
        stream.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"pong");
        server.await.unwrap();
    }
}
