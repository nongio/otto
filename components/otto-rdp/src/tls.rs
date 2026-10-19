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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use tokio_rustls::TlsAcceptor;

/// `$XDG_STATE_HOME/otto-rdp` (or `~/.local/state/otto-rdp`).
fn state_dir() -> anyhow::Result<PathBuf> {
    let base = otto_foundations::xdg::state_home().context("no absolute XDG_STATE_HOME or HOME")?;
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
    write_private(&key_path, &key_der)?;
    tracing::info!(
        "generated self-signed TLS certificate for '{host}' at {}",
        dir.display()
    );
    Ok((cert_der, key_der))
}

/// Write `bytes` to `path` readable by its owner alone, from the moment it
/// exists: the file is created `0600`, and one left over from before is made
/// so before anything is written into it.
fn write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_key_is_never_readable_by_others() {
        let dir = std::env::temp_dir().join(format!("otto-rdp-tls-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("key.der");
        write_private(&path, b"secret").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        // One left world-readable from before is tightened too.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, b"new").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(fs::read(&path).unwrap(), b"new");
        fs::remove_dir_all(&dir).unwrap();
    }
}
