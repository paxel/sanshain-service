//! Webhook deliveries over TLS trust the process trust store.
//!
//! A CA + receiver certificate (SAN `127.0.0.1`) is generated with rcgen and the
//! CA is installed through `EXTRA_CA_CERTS_DIR` before the sender is built. A
//! private CA is in no platform store, so a delivery that reaches the receiver
//! proves the sender used the process trust store. Its own binary because the
//! trust store is a process-wide `OnceLock`.

#![expect(
    unsafe_code,
    reason = "edition 2024: std::env::set_var/remove_var are unsafe"
)]

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, SanType};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sanshain_service::domain::ports::{WebhookRequest, WebhookSender};
use sanshain_service::infrastructure::tls;
use sanshain_service::infrastructure::webhook_sender::ReqwestWebhookSender;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[cfg(test)]
fn generate_certs() -> (String, String, String) {
    let ca_key = KeyPair::generate().expect("ca key");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("ca params");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "Sanshain Webhook Test CA");
    let ca_cert = ca_params.self_signed(&ca_key).expect("self-signed CA");

    let srv_key = KeyPair::generate().expect("receiver key");
    let mut srv_params = CertificateParams::new(Vec::<String>::new()).expect("receiver params");
    srv_params.subject_alt_names = vec![SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST))];
    let srv_cert = srv_params
        .signed_by(&srv_key, &Issuer::from_params(&ca_params, &ca_key))
        .expect("sign receiver cert");

    (ca_cert.pem(), srv_cert.pem(), srv_key.serialize_pem())
}

/// A one-shot HTTPS receiver: completes the handshake, reads the request and
/// answers `204`. Returns its port.
#[cfg(test)]
async fn start_receiver(srv_pem: &str, key_pem: &str) -> u16 {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(srv_pem.as_bytes())
        .collect::<Result<_, _>>()
        .expect("receiver cert");
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).expect("receiver key");
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("protocol versions")
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .expect("server config");
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut tls = acceptor.accept(stream).await.expect("handshake");
        let mut buf = vec![0u8; 8192];
        let _ = tls.read(&mut buf).await;
        tls.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .expect("answer");
        let _ = tls.shutdown().await;
    });
    port
}

#[tokio::test]
async fn a_delivery_reaches_a_receiver_signed_by_an_extra_ca() {
    let (ca_pem, srv_pem, key_pem) = generate_certs();

    // Installed BEFORE the sender is built, so this init wins the OnceLock.
    let ca_dir = std::env::temp_dir().join("sanshain_webhook_tls_ca");
    let _ = std::fs::remove_dir_all(&ca_dir);
    std::fs::create_dir_all(&ca_dir).expect("ca dir");
    std::fs::write(ca_dir.join("ca.crt"), &ca_pem).expect("write ca");
    unsafe {
        std::env::set_var(tls::EXTRA_CA_CERTS_DIR_ENV, &ca_dir);
    }
    tls::init_from_env().expect("trust store init");

    let port = start_receiver(&srv_pem, &key_pem).await;
    let sender = ReqwestWebhookSender::new(Duration::from_secs(10)).expect("sender");
    let response = sender
        .send(WebhookRequest {
            url: format!("https://127.0.0.1:{port}/hook"),
            body: "{}".to_string(),
            headers: Vec::new(),
        })
        .await
        .expect("the receiver's certificate must be trusted through EXTRA_CA_CERTS_DIR");

    assert_eq!(response.status, 204);
}
