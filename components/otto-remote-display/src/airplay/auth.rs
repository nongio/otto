/// HAP (HomeKit Accessory Protocol) pair-verify for AirPlay.
///
/// Protocol flow:
/// 1. Client generates ephemeral X25519 keypair
/// 2. POST /pair-verify with TLV { SeqNo=1, PublicKey=client_x25519_pub }
/// 3. Apple TV responds with TLV { SeqNo=2, PublicKey=atv_x25519_pub, EncryptedData }
/// 4. Client derives shared secret via X25519(client_priv, atv_pub)
/// 5. Client derives session key via HKDF-SHA512(shared, salt="Pair-Verify-Encrypt-Salt", info="Pair-Verify-Encrypt-Info")
/// 6. Client decrypts EncryptedData with ChaCha20-Poly1305(session_key, nonce="PV-Msg02")
/// 7. Decrypted TLV contains { Identifier=atv_id, Signature }
/// 8. Client verifies signature: Ed25519_verify(atv_ltpk, atv_x25519_pub || atv_id || client_x25519_pub)
/// 9. Client signs: Ed25519_sign(client_ltsk, client_x25519_pub || client_id || atv_x25519_pub)
/// 10. Client encrypts TLV { Identifier=client_id, Signature } with ChaCha20-Poly1305(session_key, nonce="PV-Msg03")
/// 11. POST /pair-verify with TLV { SeqNo=3, EncryptedData }
/// 12. Apple TV responds 200 OK — connection is verified

use anyhow::{Context, Result};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use sha2::Sha512;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{debug, info};
use x25519_dalek::{EphemeralSecret, PublicKey as X25519PublicKey};

use super::tlv::{TlvTag, TlvMap};

/// Stored credentials from a previous pair-setup (via pyatv or equivalent).
#[derive(Clone)]
pub struct HapCredentials {
    /// Ed25519 long-term signing key (32 bytes)
    pub ltsk: [u8; 32],
    /// Ed25519 long-term public key (32 bytes)
    pub ltpk: [u8; 32],
    /// Apple TV device identifier (UUID string as bytes)
    pub atv_id: Vec<u8>,
    /// Client identifier (UUID string as bytes)
    pub client_id: Vec<u8>,
}

impl HapCredentials {
    /// Parse credentials from the colon-separated hex string format used by pyatv.
    /// Format: ltpk_hex:ltsk_hex:atv_id_hex:client_id_hex
    pub fn from_pyatv_string(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 4 {
            anyhow::bail!("Expected 4 colon-separated hex fields, got {}", parts.len());
        }

        let ltpk_bytes = hex::decode(parts[0]).context("Invalid ltpk hex")?;
        let ltsk_bytes = hex::decode(parts[1]).context("Invalid ltsk hex")?;
        let atv_id = hex::decode(parts[2]).context("Invalid atv_id hex")?;
        let client_id = hex::decode(parts[3]).context("Invalid client_id hex")?;

        let ltpk: [u8; 32] = ltpk_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("ltpk must be 32 bytes"))?;
        let ltsk: [u8; 32] = ltsk_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("ltsk must be 32 bytes"))?;

        Ok(Self {
            ltsk,
            ltpk,
            atv_id,
            client_id,
        })
    }
}

/// Perform HAP pair-verify on a TCP connection.
/// Returns the shared secret for optional transport encryption.
pub async fn pair_verify(conn: &mut TcpStream, creds: &HapCredentials) -> Result<Vec<u8>> {
    // Generate ephemeral X25519 keypair
    let client_secret = EphemeralSecret::random_from_rng(rand::thread_rng());
    let client_public = X25519PublicKey::from(&client_secret);
    let client_pub_bytes = client_public.as_bytes().to_vec();

    // Step 1: Send our public key
    let step1_tlv = TlvMap::new()
        .with(TlvTag::SeqNo, vec![0x01])
        .with(TlvTag::PublicKey, client_pub_bytes.clone());

    let response = post_pair_verify(conn, &step1_tlv.encode()).await?;
    let resp_tlv = TlvMap::decode(&response)?;

    // Step 2: Process Apple TV's response
    let atv_pub_bytes = resp_tlv
        .get(TlvTag::PublicKey)
        .context("Missing PublicKey in pair-verify response")?;
    let encrypted_data = resp_tlv
        .get(TlvTag::EncryptedData)
        .context("Missing EncryptedData in pair-verify response")?;

    debug!("Received ATV public key ({} bytes)", atv_pub_bytes.len());

    // Derive shared secret
    let atv_pub_array: [u8; 32] = atv_pub_bytes
        .clone()
        .try_into()
        .map_err(|_| anyhow::anyhow!("ATV public key must be 32 bytes"))?;
    let atv_x25519_pub = X25519PublicKey::from(atv_pub_array);
    let shared_secret = client_secret.diffie_hellman(&atv_x25519_pub);
    let shared_bytes = shared_secret.as_bytes().to_vec();

    // Derive session key for verify encryption
    let session_key = hkdf_derive(
        "Pair-Verify-Encrypt-Salt",
        "Pair-Verify-Encrypt-Info",
        &shared_bytes,
    )?;

    // Decrypt the ATV's proof
    let cipher = ChaCha20Poly1305::new_from_slice(&session_key)
        .map_err(|e| anyhow::anyhow!("ChaCha20 init failed: {}", e))?;
    let nonce = pad_nonce(b"PV-Msg02");
    let decrypted = cipher
        .decrypt(&nonce, encrypted_data.as_ref())
        .map_err(|e| anyhow::anyhow!("Decrypt PV-Msg02 failed: {}", e))?;

    let inner_tlv = TlvMap::decode(&decrypted)?;
    let atv_identifier = inner_tlv
        .get(TlvTag::Identifier)
        .context("Missing Identifier in decrypted data")?;
    let atv_signature = inner_tlv
        .get(TlvTag::Signature)
        .context("Missing Signature in decrypted data")?;

    // Verify Apple TV's identity matches stored credentials
    if atv_identifier != &creds.atv_id {
        anyhow::bail!(
            "ATV identifier mismatch: expected {:?}, got {:?}",
            String::from_utf8_lossy(&creds.atv_id),
            String::from_utf8_lossy(atv_identifier)
        );
    }

    // Verify Apple TV's signature: sign(atv_ltsk, atv_x25519_pub || atv_id || client_x25519_pub)
    let mut verify_info = Vec::new();
    verify_info.extend_from_slice(atv_pub_bytes);
    verify_info.extend_from_slice(atv_identifier);
    verify_info.extend_from_slice(&client_pub_bytes);

    let atv_verifying_key = VerifyingKey::from_bytes(&creds.ltpk)
        .map_err(|e| anyhow::anyhow!("Invalid ATV ltpk: {}", e))?;
    let signature = ed25519_dalek::Signature::from_slice(atv_signature)
        .map_err(|e| anyhow::anyhow!("Invalid signature format: {}", e))?;
    atv_verifying_key
        .verify(&verify_info, &signature)
        .map_err(|e| anyhow::anyhow!("ATV signature verification failed: {}", e))?;

    info!("Apple TV identity verified");

    // Step 3: Send our proof
    // Sign: sign(client_ltsk, client_x25519_pub || client_id || atv_x25519_pub)
    let mut sign_info = Vec::new();
    sign_info.extend_from_slice(&client_pub_bytes);
    sign_info.extend_from_slice(&creds.client_id);
    sign_info.extend_from_slice(atv_pub_bytes);

    let signing_key = SigningKey::from_bytes(&creds.ltsk);
    let our_signature = signing_key.sign(&sign_info);

    let proof_tlv = TlvMap::new()
        .with(TlvTag::Identifier, creds.client_id.clone())
        .with(TlvTag::Signature, our_signature.to_bytes().to_vec());

    let nonce3 = pad_nonce(b"PV-Msg03");
    let encrypted_proof = cipher
        .encrypt(&nonce3, proof_tlv.encode().as_ref())
        .map_err(|e| anyhow::anyhow!("Encrypt PV-Msg03 failed: {}", e))?;

    let step3_tlv = TlvMap::new()
        .with(TlvTag::SeqNo, vec![0x03])
        .with(TlvTag::EncryptedData, encrypted_proof);

    let _response = post_pair_verify(conn, &step3_tlv.encode()).await?;

    info!("Pair-verify completed successfully");
    Ok(shared_bytes)
}

/// Derive encryption keys from the shared secret for the control channel.
pub fn derive_encryption_keys(shared_secret: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let output_key = hkdf_derive(
        "Control-Salt",
        "Control-Write-Encryption-Key",
        shared_secret,
    )?;
    let input_key = hkdf_derive(
        "Control-Salt",
        "Control-Read-Encryption-Key",
        shared_secret,
    )?;
    Ok((output_key, input_key))
}

/// HAP framing: encrypt data into 1024-byte blocks.
/// Each block: [2-byte LE length][ChaCha20-Poly1305(block, aad=length_bytes, nonce=counter)]
pub fn hap_encrypt(data: &[u8], key: &[u8], start_counter: u64) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| anyhow::anyhow!("ChaCha20 init: {}", e))?;

    let mut output = Vec::new();
    let mut counter = start_counter;
    let mut offset = 0;

    while offset < data.len() {
        let end = (offset + 1024).min(data.len());
        let frame = &data[offset..end];
        let length_bytes = (frame.len() as u16).to_le_bytes();

        let nonce = counter_to_nonce(counter);
        let encrypted = cipher
            .encrypt(&nonce, chacha20poly1305::aead::Payload { msg: frame, aad: &length_bytes })
            .map_err(|e| anyhow::anyhow!("HAP encrypt failed: {}", e))?;

        output.extend_from_slice(&length_bytes);
        output.extend_from_slice(&encrypted);

        counter += 1;
        offset = end;
    }

    Ok(output)
}

/// HAP framing: decrypt data from 1024-byte blocks.
pub fn hap_decrypt(data: &[u8], key: &[u8], start_counter: u64) -> Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|e| anyhow::anyhow!("ChaCha20 init: {}", e))?;

    let mut output = Vec::new();
    let mut counter = start_counter;
    let mut offset = 0;

    while offset + 2 < data.len() {
        let length = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
        offset += 2;

        let block_len = length + 16; // 16 byte auth tag
        if offset + block_len > data.len() {
            break;
        }

        let block = &data[offset..offset + block_len];
        let length_bytes = (length as u16).to_le_bytes();
        let nonce = counter_to_nonce(counter);

        let decrypted = cipher
            .decrypt(&nonce, chacha20poly1305::aead::Payload { msg: block, aad: &length_bytes })
            .map_err(|e| anyhow::anyhow!("HAP decrypt failed at counter {}: {}", counter, e))?;

        output.extend_from_slice(&decrypted);
        counter += 1;
        offset += block_len;
    }

    Ok(output)
}

fn counter_to_nonce(counter: u64) -> Nonce {
    let mut nonce_bytes = [0u8; 12];
    // 8-byte LE counter padded with 4 leading zero bytes
    nonce_bytes[4..].copy_from_slice(&counter.to_le_bytes());
    *Nonce::from_slice(&nonce_bytes)
}

fn hkdf_derive(salt: &str, info: &str, ikm: &[u8]) -> Result<Vec<u8>> {
    let hk = Hkdf::<Sha512>::new(Some(salt.as_bytes()), ikm);
    let mut okm = vec![0u8; 32];
    hk.expand(info.as_bytes(), &mut okm)
        .map_err(|e| anyhow::anyhow!("HKDF expand failed: {}", e))?;
    Ok(okm)
}

fn pad_nonce(nonce: &[u8]) -> Nonce {
    let mut padded = [0u8; 12];
    let start = 12 - nonce.len();
    padded[start..].copy_from_slice(nonce);
    *Nonce::from_slice(&padded)
}

async fn post_pair_verify(conn: &mut TcpStream, body: &[u8]) -> Result<Vec<u8>> {
    let request = format!(
        "POST /pair-verify HTTP/1.1\r\n\
         User-Agent: AirPlay/320.20\r\n\
         Connection: keep-alive\r\n\
         X-Apple-HKP: 3\r\n\
         Content-Type: application/octet-stream\r\n\
         Content-Length: {}\r\n\
         \r\n",
        body.len(),
    );

    conn.write_all(request.as_bytes()).await?;
    conn.write_all(body).await?;

    // Read response
    let mut buf = vec![0u8; 8192];
    let n = conn.read(&mut buf).await?;
    let response = &buf[..n];

    // Parse HTTP response — find body after \r\n\r\n
    let header_end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("No HTTP header end in pair-verify response")?;

    let headers = String::from_utf8_lossy(&response[..header_end]);
    let status_line = headers.lines().next().unwrap_or("");
    debug!("pair-verify response: {}", status_line);

    if !status_line.contains("200") {
        anyhow::bail!("pair-verify failed: {}", status_line);
    }

    let body_start = header_end + 4;
    Ok(response[body_start..].to_vec())
}
