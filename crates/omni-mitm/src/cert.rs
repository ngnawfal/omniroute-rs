// Pure-Rust X.509 v3 Root CA & Dynamic Leaf Certificate Minting using `ring` ECDSA P-256.
// Zero OpenSSL dependency — generates standard DER/PEM certificates accepted by rustls & OS trust stores.

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

// ── Minimal ASN.1 DER primitives ────────────────────────────────────

fn der_len(len: usize) -> Vec<u8> {
    if len < 0x80 {
        vec![len as u8]
    } else if len <= 0xff {
        vec![0x81, len as u8]
    } else if len <= 0xffff {
        vec![0x82, (len >> 8) as u8, (len & 0xff) as u8]
    } else {
        vec![
            0x83,
            ((len >> 16) & 0xff) as u8,
            ((len >> 8) & 0xff) as u8,
            (len & 0xff) as u8,
        ]
    }
}

fn der_tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 4 + content.len());
    out.push(tag);
    out.extend_from_slice(&der_len(content.len()));
    out.extend_from_slice(content);
    out
}

fn der_seq(parts: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = parts.iter().flat_map(|p| p.iter().copied()).collect();
    der_tlv(0x30, &body)
}

fn der_set(parts: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = parts.iter().flat_map(|p| p.iter().copied()).collect();
    der_tlv(0x31, &body)
}

fn der_int_bytes(raw: &[u8]) -> Vec<u8> {
    let mut trimmed = raw;
    while trimmed.len() > 1 && trimmed[0] == 0 {
        trimmed = &trimmed[1..];
    }
    let mut val = Vec::with_capacity(trimmed.len() + 1);
    if trimmed.is_empty() {
        val.push(0);
    } else {
        if trimmed[0] & 0x80 != 0 {
            val.push(0);
        }
        val.extend_from_slice(trimmed);
    }
    der_tlv(0x02, &val)
}

fn der_int_u64(v: u64) -> Vec<u8> {
    der_int_bytes(&v.to_be_bytes())
}

fn der_bit_string(data: &[u8]) -> Vec<u8> {
    let mut val = Vec::with_capacity(data.len() + 1);
    val.push(0); // unused bits = 0
    val.extend_from_slice(data);
    der_tlv(0x03, &val)
}

fn der_octet_string(data: &[u8]) -> Vec<u8> {
    der_tlv(0x04, data)
}

fn der_oid(arcs: &[u32]) -> Vec<u8> {
    assert!(arcs.len() >= 2);
    let mut out = vec![(arcs[0] * 40 + arcs[1]) as u8];
    for &arc in &arcs[2..] {
        if arc < 0x80 {
            out.push(arc as u8);
        } else {
            let mut buf = Vec::new();
            let mut v = arc;
            buf.push((v & 0x7f) as u8);
            v >>= 7;
            while v > 0 {
                buf.push(((v & 0x7f) as u8) | 0x80);
                v >>= 7;
            }
            buf.reverse();
            out.extend_from_slice(&buf);
        }
    }
    der_tlv(0x06, &out)
}

fn der_utf8(s: &str) -> Vec<u8> {
    der_tlv(0x0c, s.as_bytes())
}

fn der_utc_time(s: &str) -> Vec<u8> {
    der_tlv(0x17, s.as_bytes())
}

fn der_bool(b: bool) -> Vec<u8> {
    der_tlv(0x01, &[if b { 0xff } else { 0x00 }])
}

fn der_explicit(tag_num: u8, inner: Vec<u8>) -> Vec<u8> {
    der_tlv(0xa0 | tag_num, &inner)
}

fn der_name_cn(cn: &str, org: &str) -> Vec<u8> {
    let rdn_o = der_set(&[der_seq(&[der_oid(&[2, 5, 4, 10]), der_utf8(org)])]);
    let rdn_cn = der_set(&[der_seq(&[der_oid(&[2, 5, 4, 3]), der_utf8(cn)])]);
    der_seq(&[rdn_o, rdn_cn])
}

fn alg_ecdsa_sha256() -> Vec<u8> {
    // 1.2.840.10045.4.3.2 (ecdsa-with-SHA256)
    der_seq(&[der_oid(&[1, 2, 840, 10045, 4, 3, 2])])
}

fn spki_p256(pub_key_uncompressed: &[u8]) -> Vec<u8> {
    // id-ecPublicKey (1.2.840.10045.2.1), prime256v1 (1.2.840.10045.3.1.7)
    let alg = der_seq(&[
        der_oid(&[1, 2, 840, 10045, 2, 1]),
        der_oid(&[1, 2, 840, 10045, 3, 1, 7]),
    ]);
    der_seq(&[alg, der_bit_string(pub_key_uncompressed)])
}

fn extension_entry(oid: &[u32], critical: bool, value_der: Vec<u8>) -> Vec<u8> {
    let mut parts = vec![der_oid(oid)];
    if critical {
        parts.push(der_bool(true));
    }
    parts.push(der_octet_string(&value_der));
    der_seq(&parts)
}

fn encode_pem(label: &str, der: &[u8]) -> String {
    let b64 = B64.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn decode_pem(pem: &str) -> Result<Vec<u8>, String> {
    let body: String = pem
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("-----"))
        .collect();
    B64.decode(body).map_err(|e| format!("invalid PEM base64: {e}"))
}

fn random_serial(rng: &SystemRandom) -> Vec<u8> {
    let mut bytes = [0u8; 16];
    let _ = ring::rand::SecureRandom::fill(rng, &mut bytes);
    bytes[0] &= 0x7f; // positive integer
    bytes[0] |= 0x01; // non-zero
    bytes.to_vec()
}

fn build_signed_cert(
    subject_cn: &str,
    issuer_cn: &str,
    subject_pub_key: &[u8],
    issuer_key: &EcdsaKeyPair,
    is_ca: bool,
    san_hosts: &[&str],
    valid_days: i64,
    rng: &SystemRandom,
) -> Result<Vec<u8>, String> {
    let now = chrono::Utc::now();
    let not_before = (now - chrono::Duration::minutes(5))
        .format("%y%m%d%H%M%SZ")
        .to_string();
    let not_after = (now + chrono::Duration::days(valid_days))
        .format("%y%m%d%H%M%SZ")
        .to_string();

    let mut exts = Vec::new();
    // BasicConstraints (2.5.29.19)
    let bc = if is_ca {
        der_seq(&[der_bool(true)])
    } else {
        der_seq(&[])
    };
    exts.push(extension_entry(&[2, 5, 29, 19], true, bc));

    // KeyUsage (2.5.29.15)
    if is_ca {
        // keyCertSign (bit 5) | cRLSign (bit 6) -> 0x06 with 1 unused bit
        exts.push(extension_entry(
            &[2, 5, 29, 15],
            true,
            der_tlv(0x03, &[0x01, 0x06]),
        ));
    } else {
        // digitalSignature (bit 0) -> 0x80 with 7 unused bits
        exts.push(extension_entry(
            &[2, 5, 29, 15],
            true,
            der_tlv(0x03, &[0x07, 0x80]),
        ));
        // ExtKeyUsage (2.5.29.37): id-kp-serverAuth (1.3.6.1.5.5.7.3.1)
        exts.push(extension_entry(
            &[2, 5, 29, 37],
            false,
            der_seq(&[der_oid(&[1, 3, 6, 1, 5, 5, 7, 3, 1])]),
        ));
    }

    // SubjectAltName (2.5.29.17)
    if !san_hosts.is_empty() {
        let mut names = Vec::new();
        for host in san_hosts {
            if let Ok(ip) = host.parse::<std::net::IpAddr>() {
                match ip {
                    std::net::IpAddr::V4(v4) => names.push(der_tlv(0x87, &v4.octets())),
                    std::net::IpAddr::V6(v6) => names.push(der_tlv(0x87, &v6.octets())),
                }
            } else {
                names.push(der_tlv(0x82, host.as_bytes()));
            }
        }
        exts.push(extension_entry(&[2, 5, 29, 17], false, der_seq(&names)));
    }

    let tbs = der_seq(&[
        der_explicit(0, der_int_u64(2)), // v3 = 2
        der_int_bytes(&random_serial(rng)),
        alg_ecdsa_sha256(),
        der_name_cn(issuer_cn, "OmniRoute MITM"),
        der_seq(&[der_utc_time(&not_before), der_utc_time(&not_after)]),
        der_name_cn(subject_cn, "OmniRoute MITM"),
        spki_p256(subject_pub_key),
        der_explicit(3, der_seq(&exts)),
    ]);

    let sig = issuer_key
        .sign(rng, &tbs)
        .map_err(|_| "ECDSA signing failed".to_string())?;
    Ok(der_seq(&[tbs, alg_ecdsa_sha256(), der_bit_string(sig.as_ref())]))
}

// ── Certificate Authority + Leaf Cache ──────────────────────────────

pub const ROOT_CA_CN: &str = "OmniRoute Local MITM Root CA";

#[derive(Clone)]
pub struct LeafIdentity {
    pub cert_chain: Vec<CertificateDer<'static>>,
    pub private_key_pkcs8: Vec<u8>,
}

pub struct CertAuthority {
    ca_cert_der: Vec<u8>,
    ca_key_pkcs8: Vec<u8>,
    ca_keypair: EcdsaKeyPair,
    rng: SystemRandom,
    leaf_cache: RwLock<HashMap<String, Arc<LeafIdentity>>>,
}

impl CertAuthority {
    pub fn generate() -> Result<Self, String> {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng)
            .map_err(|_| "failed to generate ECDSA P-256 CA key".to_string())?;
        let keypair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
                .map_err(|_| "failed to parse generated CA key".to_string())?;
        let ca_cert_der = build_signed_cert(
            ROOT_CA_CN,
            ROOT_CA_CN,
            keypair.public_key().as_ref(),
            &keypair,
            true,
            &[],
            3650,
            &rng,
        )?;
        Ok(Self {
            ca_cert_der,
            ca_key_pkcs8: pkcs8.as_ref().to_vec(),
            ca_keypair: keypair,
            rng,
            leaf_cache: RwLock::new(HashMap::new()),
        })
    }

    pub fn load_or_create(cert_dir: &Path) -> Result<Self, String> {
        let cert_path = cert_dir.join("ca.crt");
        let key_path = cert_dir.join("ca.key");
        if cert_path.exists() && key_path.exists() {
            let cert_pem = std::fs::read_to_string(&cert_path)
                .map_err(|e| format!("read ca.crt: {e}"))?;
            let key_pem = std::fs::read_to_string(&key_path)
                .map_err(|e| format!("read ca.key: {e}"))?;
            let ca_cert_der = decode_pem(&cert_pem)?;
            let ca_key_pkcs8 = decode_pem(&key_pem)?;
            let rng = SystemRandom::new();
            let ca_keypair =
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &ca_key_pkcs8, &rng)
                    .map_err(|_| "failed to load CA keypair from ca.key".to_string())?;
            return Ok(Self {
                ca_cert_der,
                ca_key_pkcs8,
                ca_keypair,
                rng,
                leaf_cache: RwLock::new(HashMap::new()),
            });
        }
        let ca = Self::generate()?;
        let _ = std::fs::create_dir_all(cert_dir);
        let _ = std::fs::write(&cert_path, ca.ca_cert_pem());
        let _ = std::fs::write(&key_path, ca.ca_key_pem());
        Ok(ca)
    }

    pub fn ca_cert_der(&self) -> &[u8] {
        &self.ca_cert_der
    }

    pub fn ca_cert_pem(&self) -> String {
        encode_pem("CERTIFICATE", &self.ca_cert_der)
    }

    pub fn ca_key_pem(&self) -> String {
        encode_pem("PRIVATE KEY", &self.ca_key_pkcs8)
    }

    pub fn ca_fingerprint_sha256(&self) -> String {
        let mut h = Sha256::new();
        h.update(&self.ca_cert_der);
        hex::encode(h.finalize())
    }

    pub fn clear_leaf_cache(&self) {
        self.leaf_cache.write().unwrap().clear();
    }

    pub fn cached_leaf_count(&self) -> usize {
        self.leaf_cache.read().unwrap().len()
    }

    /// Mint (or retrieve from cache) a leaf TLS identity signed by the Root CA for `hostname`.
    pub fn issue_leaf_for_host(&self, hostname: &str) -> Result<Arc<LeafIdentity>, String> {
        let norm = hostname.trim().to_ascii_lowercase();
        if let Some(hit) = self.leaf_cache.read().unwrap().get(&norm).cloned() {
            return Ok(hit);
        }

        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &self.rng)
            .map_err(|_| "failed to generate leaf key".to_string())?;
        let leaf_kp =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &self.rng)
                .map_err(|_| "failed to parse leaf key".to_string())?;
        let leaf_der = build_signed_cert(
            &norm,
            ROOT_CA_CN,
            leaf_kp.public_key().as_ref(),
            &self.ca_keypair,
            false,
            &[norm.as_str()],
            365,
            &self.rng,
        )?;
        let identity = Arc::new(LeafIdentity {
            cert_chain: vec![
                CertificateDer::from(leaf_der),
                CertificateDer::from(self.ca_cert_der.clone()),
            ],
            private_key_pkcs8: pkcs8.as_ref().to_vec(),
        });
        let mut cache = self.leaf_cache.write().unwrap();
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(norm, identity.clone());
        Ok(identity)
    }

    pub fn build_server_config_for_host(
        &self,
        hostname: &str,
    ) -> Result<Arc<rustls::ServerConfig>, String> {
        let leaf = self.issue_leaf_for_host(hostname)?;
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf.private_key_pkcs8.clone()));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("rustls protocol versions: {e}"))?
            .with_no_client_auth()
            .with_single_cert(leaf.cert_chain.clone(), key)
            .map_err(|e| format!("rustls server cert config: {e}"))?;
        Ok(Arc::new(cfg))
    }
}
