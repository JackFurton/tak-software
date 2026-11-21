//! A small certificate authority for the TLS port, and ATAK connection packages signed by it.
//!
//! `init` creates the CA and the server certificate. `client` issues one certificate per device
//! and bundles it as a data package ATAK imports in one step (Import Manager, or drag the zip
//! onto the device): the CA as a truststore, the device's key as a .p12, and the server entry.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use chrono::{Datelike, Utc};
use p12_keystore::{
    Certificate as P12Cert, EncryptionAlgorithm, KeyStore, KeyStoreEntry, MacAlgorithm, PrivateKey,
    PrivateKeyChain,
};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::WebPkiClientVerifier;
use zip::write::SimpleFileOptions;

/// ATAK's conventional keystore password; the files protect nothing on their own, the device
/// does. Matches what TAK Server's scripts and every ATAK tutorial use.
pub const P12_PASSWORD: &str = "atakatak";

const CA_CERT: &str = "ca.pem";
const CA_KEY: &str = "ca-key.pem";
const SERVER_CERT: &str = "server.pem";
const SERVER_KEY: &str = "server-key.pem";

/// Creates the CA and a server certificate valid for `hosts` (names or IPs clients dial).
pub fn init(dir: &Path, hosts: &[String]) -> Result<()> {
    ensure!(!hosts.is_empty(), "give at least one --host the server is reached at");
    ensure!(!dir.join(CA_KEY).exists(), "{} already has a CA; remove it to start over", dir.display());
    fs::create_dir_all(dir)?;

    let mut ca = CertificateParams::new(Vec::new())?;
    ca.distinguished_name.push(DnType::CommonName, "tak-software CA");
    ca.distinguished_name.push(DnType::OrganizationName, "tak-software");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    valid_for_years(&mut ca, 10);
    let ca_key = KeyPair::generate()?;
    let ca_cert = ca.self_signed(&ca_key)?;
    let ca_key_pem = ca_key.serialize_pem();
    let issuer = Issuer::new(ca, ca_key);

    let mut server = CertificateParams::new(hosts.to_vec())?;
    server.distinguished_name.push(DnType::CommonName, &hosts[0]);
    server.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    server.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    valid_for_years(&mut server, 2);
    let server_key = KeyPair::generate()?;
    let server_cert = server.signed_by(&server_key, &issuer)?;

    write(&dir.join(CA_CERT), ca_cert.pem().as_bytes(), false)?;
    write(&dir.join(CA_KEY), ca_key_pem.as_bytes(), true)?;
    write(&dir.join(SERVER_CERT), server_cert.pem().as_bytes(), false)?;
    write(&dir.join(SERVER_KEY), server_key.serialize_pem().as_bytes(), true)?;
    Ok(())
}

/// Issues a client certificate for `name` and writes `<name>.zip`, a data package that points
/// ATAK at `connect` (`host:port`). Returns the package path.
pub fn client(dir: &Path, name: &str, connect: &str) -> Result<PathBuf> {
    ensure!(
        (1..=64).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)),
        "client name must be 1-64 of A-Z a-z 0-9 - _"
    );
    let ca_pem = fs::read_to_string(dir.join(CA_CERT))
        .with_context(|| format!("no CA in {}; run `takserver certs init` first", dir.display()))?;
    let ca_key = KeyPair::from_pem(&fs::read_to_string(dir.join(CA_KEY))?)?;
    let issuer = Issuer::from_ca_cert_pem(&ca_pem, ca_key)?;
    let ca_der = CertificateDer::from_pem_slice(ca_pem.as_bytes())?;

    let mut params = CertificateParams::new(Vec::new())?;
    params.distinguished_name.push(DnType::CommonName, name);
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    valid_for_years(&mut params, 2);
    let key = KeyPair::generate()?;
    let cert = params.signed_by(&key, &issuer)?;

    let clients = dir.join("clients");
    fs::create_dir_all(&clients)?;
    write(&clients.join(format!("{name}.pem")), cert.pem().as_bytes(), false)?;
    write(&clients.join(format!("{name}-key.pem")), key.serialize_pem().as_bytes(), true)?;

    let mut user = KeyStore::new();
    let chain = PrivateKeyChain::new(
        cert.der().to_vec(),
        PrivateKey::from_der(key.serialized_der())?,
        [P12Cert::from_der(cert.der())?, P12Cert::from_der(&ca_der)?],
    );
    user.add_entry(name, KeyStoreEntry::PrivateKeyChain(chain));
    let mut trust = KeyStore::new();
    trust.add_entry("tak-software-ca", KeyStoreEntry::Certificate(P12Cert::from_der(&ca_der)?));

    let path = clients.join(format!("{name}.zip"));
    let package = data_package(name, connect, &legacy_p12(&user)?, &legacy_p12(&trust)?)?;
    write(&path, &package, true)?;
    Ok(path)
}

/// TLS settings for the streaming port: only clients holding a certificate from our CA get in.
pub fn server_config(dir: &Path) -> Result<Arc<rustls::ServerConfig>> {
    let read = |name: &str| {
        let path = dir.join(name);
        fs::read(&path)
            .with_context(|| format!("reading {}; run `takserver certs init` to create it", path.display()))
    };
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from_pem_slice(&read(CA_CERT)?)?)?;
    let chain = CertificateDer::pem_slice_iter(&read(SERVER_CERT)?).collect::<Result<Vec<_>, _>>()?;
    let key = PrivateKeyDer::from_pem_slice(&read(SERVER_KEY)?)?;

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone()).build()?;
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain, key)?;
    Ok(Arc::new(config))
}

/// Common name of the leaf certificate a TLS client presented.
pub fn peer_name(certs: &[CertificateDer<'_>]) -> Option<String> {
    let (_, cert) = x509_parser::parse_x509_certificate(certs.first()?).ok()?;
    cert.subject().iter_common_name().next()?.as_str().ok().map(String::from)
}

// Android's PKCS#12 loader (and so ATAK) rejects the AES/PBES2 files OpenSSL 3 and most
// libraries now write by default; the old 3DES + SHA-1 MAC form imports everywhere.
fn legacy_p12(store: &KeyStore) -> Result<Vec<u8>> {
    Ok(store
        .writer(P12_PASSWORD)
        .encryption_algorithm(EncryptionAlgorithm::PbeWithShaAnd3KeyTripleDesCbc)
        .mac_algorithm(MacAlgorithm::HmacSha1)
        .write()?)
}

fn data_package(name: &str, connect: &str, user_p12: &[u8], trust_p12: &[u8]) -> Result<Vec<u8>> {
    let Some((host, port)) = connect.rsplit_once(':') else { bail!("--connect must be host:port") };
    let e = cot::xml_escape;
    let pref = format!(
        r#"<?xml version='1.0' encoding='ASCII' standalone='yes'?>
<preferences>
  <preference version="1" name="cot_streams">
    <entry key="count" class="class java.lang.Integer">1</entry>
    <entry key="description0" class="class java.lang.String">tak-software</entry>
    <entry key="enabled0" class="class java.lang.Boolean">true</entry>
    <entry key="connectString0" class="class java.lang.String">{host}:{port}:ssl</entry>
  </preference>
  <preference version="1" name="com.atakmap.app_preferences">
    <entry key="displayServerConnectionWidget" class="class java.lang.Boolean">true</entry>
    <entry key="caLocation" class="class java.lang.String">cert/truststore-root.p12</entry>
    <entry key="caPassword" class="class java.lang.String">{P12_PASSWORD}</entry>
    <entry key="certificateLocation" class="class java.lang.String">cert/{name}.p12</entry>
    <entry key="clientPassword" class="class java.lang.String">{P12_PASSWORD}</entry>
  </preference>
</preferences>
"#,
        host = e(host),
        port = e(port),
    );
    let manifest = format!(
        r#"<MissionPackageManifest version="2">
  <Configuration>
    <Parameter name="uid" value="tak-software-{name}-{stamp}"/>
    <Parameter name="name" value="{name}.zip"/>
    <Parameter name="onReceiveDelete" value="true"/>
  </Configuration>
  <Contents>
    <Content ignore="false" zipEntry="certs/server.pref"/>
    <Content ignore="false" zipEntry="certs/truststore-root.p12"/>
    <Content ignore="false" zipEntry="certs/{name}.p12"/>
  </Contents>
</MissionPackageManifest>
"#,
        stamp = Utc::now().timestamp(),
    );

    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default();
    for (path, bytes) in [
        ("MANIFEST/manifest.xml".to_owned(), manifest.as_bytes()),
        ("certs/server.pref".to_owned(), pref.as_bytes()),
        ("certs/truststore-root.p12".to_owned(), trust_p12),
        (format!("certs/{name}.p12"), user_p12),
    ] {
        zip.start_file(path, opts)?;
        zip.write_all(bytes)?;
    }
    Ok(zip.finish()?.into_inner())
}

fn valid_for_years(params: &mut CertificateParams, years: i32) {
    // Backdated a day so a phone with a slightly slow clock doesn't see a future certificate.
    let start = Utc::now() - chrono::Duration::days(1);
    params.not_before = rcgen::date_time_ymd(start.year(), start.month() as u8, start.day() as u8);
    params.not_after =
        rcgen::date_time_ymd(start.year() + years, start.month() as u8, start.day().min(28) as u8);
}

fn write(path: &Path, bytes: &[u8], secret: bool) -> Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if secret {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = secret;
    opts.open(path)?.write_all(bytes).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issues_importable_package() {
        let dir = std::env::temp_dir().join(format!("takserver-certs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        init(&dir, &["192.168.1.155".into(), "tak.local".into()]).unwrap();
        assert!(init(&dir, &["x".into()]).is_err(), "must not overwrite an existing CA");
        server_config(&dir).unwrap();

        let zip_path = client(&dir, "viper", "192.168.1.155:8089").unwrap();
        let mut zip = zip::ZipArchive::new(fs::File::open(zip_path).unwrap()).unwrap();
        let mut read = |name: &str| {
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut zip.by_name(name).unwrap(), &mut out).unwrap();
            out
        };
        let pref = String::from_utf8(read("certs/server.pref")).unwrap();
        assert!(pref.contains("192.168.1.155:8089:ssl"));
        assert!(String::from_utf8(read("MANIFEST/manifest.xml")).unwrap().contains("certs/viper.p12"));

        let user = KeyStore::from_pkcs12(&read("certs/viper.p12"), P12_PASSWORD, Default::default()).unwrap();
        let (_, chain) = user.private_key_chain().unwrap();
        assert_eq!(chain.certs().len(), 2);
        let trust =
            KeyStore::from_pkcs12(&read("certs/truststore-root.p12"), P12_PASSWORD, Default::default())
                .unwrap();
        assert_eq!(trust.entries_len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
