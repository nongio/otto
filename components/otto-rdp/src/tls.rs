//! Self-signed TLS for the RDP listener (`--tls`).
//!
//! TLS-only clients — Windows `mstsc`, Microsoft's mobile "Windows App" —
//! refuse the plain-RDP security layer entirely. A self-signed certificate
//! is enough for them: the client shows a trust prompt on first connect.
//!
//! The certificate and key are generated once and persisted under the state
//! directory so the fingerprint stays stable across restarts — otherwise the
//! client would re-prompt (or refuse, if the cert was pinned) every run.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use tokio_rustls::TlsAcceptor;

/// `$XDG_STATE_HOME/otto-rdp` (or `~/.local/state/otto-rdp`).
fn state_dir() -> anyhow::Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .context("neither XDG_STATE_HOME nor HOME is set")?;
    Ok(base.join("otto-rdp"))
}

/// Load the persisted certificate, or generate + persist a fresh one.
fn cert_pair() -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
    let dir = state_dir()?;
    let cert_path = dir.join("cert.der");
    let key_path = dir.join("key.der");

    if let (Ok(cert), Ok(key)) = (fs::read(&cert_path), fs::read(&key_path)) {
        tracing::info!("using TLS certificate from {}", dir.display());
        return Ok((cert, key));
    }

    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "otto-rdp".into());
    let cert = rcgen::generate_simple_self_signed(vec![host.clone(), "localhost".into()])
        .context("generating self-signed certificate")?;
    let cert_der = cert.cert.der().to_vec();
    let key_der = cert.key_pair.serialize_der();

    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    fs::write(&cert_path, &cert_der)?;
    fs::write(&key_path, &key_der)?;
    // The key is private — never world-readable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
    }
    tracing::info!(
        "generated self-signed TLS certificate for '{host}' at {}",
        dir.display()
    );
    Ok((cert_der, key_der))
}

pub fn acceptor() -> anyhow::Result<TlsAcceptor> {
    let (cert_der, key_der) = cert_pair()?;
    let cert = rustls::pki_types::CertificateDer::from(cert_der);
    let key = rustls::pki_types::PrivateKeyDer::try_from(key_der)
        .map_err(|e| anyhow::anyhow!("invalid private key: {e}"))?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .context("building rustls server config")?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}
