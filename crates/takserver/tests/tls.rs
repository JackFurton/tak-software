use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cot::framing::StreamDecoder;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use takserver::{Config, certs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_util::sync::CancellationToken;

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("takserver-tls-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn connector(dir: &Path, client: Option<&str>) -> TlsConnector {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from_pem_file(dir.join("ca.pem")).unwrap()).unwrap();
    let builder =
        rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots);
    let config = match client {
        Some(name) => {
            let clients = dir.join("clients");
            let chain = vec![CertificateDer::from_pem_file(clients.join(format!("{name}.pem"))).unwrap()];
            let key = PrivateKeyDer::from_pem_file(clients.join(format!("{name}-key.pem"))).unwrap();
            builder.with_client_auth_cert(chain, key).unwrap()
        }
        None => builder.with_no_client_auth(),
    };
    TlsConnector::from(Arc::new(config))
}

#[tokio::test]
async fn tls_port_requires_a_client_certificate() {
    let dir = temp_dir();
    certs::init(&dir, &["127.0.0.1".into()]).unwrap();
    certs::client(&dir, "viper", "127.0.0.1:8089").unwrap();

    let cancel = CancellationToken::new();
    let config = Config {
        tcp: "127.0.0.1:0".parse().unwrap(),
        tls: Some(("127.0.0.1:0".parse().unwrap(), dir.clone())),
        http: "127.0.0.1:0".parse().unwrap(),
        web_root: None,
        mesh: None,
    };
    let server = takserver::start(config, cancel.clone()).await.unwrap();
    let addr = server.tls_addr.unwrap();
    let name = ServerName::try_from("127.0.0.1").unwrap();

    // With a certificate: the server's protocol offer arrives over TLS.
    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut tls = connector(&dir, Some("viper")).connect(name.clone(), tcp).await.unwrap();
    let mut decoder = StreamDecoder::default();
    let mut buf = [0u8; 4096];
    let offer = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(f) = decoder.next_frame().unwrap() {
                return f.decode().unwrap().event.unwrap();
            }
            let n = tls.read(&mut buf).await.unwrap();
            assert!(n > 0);
            decoder.extend(&buf[..n]);
        }
    })
    .await
    .unwrap();
    assert_eq!(offer.event_type, cot::takp::SUPPORT_TYPE);

    // Without one: TLS 1.3 lets the client finish its side, then the server's alert ends it.
    let tcp = TcpStream::connect(addr).await.unwrap();
    let refused = async {
        let mut tls = connector(&dir, None).connect(name, tcp).await?;
        tls.write_all(b"<event/>").await?;
        let n = tls.read(&mut buf).await?;
        Ok::<_, std::io::Error>(n)
    };
    match tokio::time::timeout(Duration::from_secs(5), refused).await.unwrap() {
        Ok(0) | Err(_) => {}
        Ok(n) => panic!("server sent {n} bytes to a client without a certificate"),
    }

    cancel.cancel();
    std::fs::remove_dir_all(&dir).unwrap();
}
