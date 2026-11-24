// Live TCP + HTTP CONNECT + Dynamic TLS Interception MITM Proxy Server.
//
// Routing architecture:
//   Client -> HTTP CONNECT host:port -> route_connection(host, bypass):
//     - Bypass      -> 200 Connection Established + zero-copy bidirectional TCP tunnel
//     - Passthrough -> 200 Connection Established + zero-copy bidirectional TCP tunnel
//     - Target      -> 200 Connection Established + dynamic TLS termination (rustls + Root CA)
//                      -> inspect HTTP/1.1 request:
//                           * if path matches target endpoint_patterns -> forward to local OmniRoute gateway!
//                           * else -> forward transparently over HTTPS to original host!

use crate::cert::CertAuthority;
use crate::{route_connection, target_matches_path, ConnectionRoute, MitmTarget};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MitmRuntimeStats {
    pub running: bool,
    pub bind_addr: Option<String>,
    pub gateway_base_url: String,
    pub started_at: Option<String>,
    pub total_connections: u64,
    pub active_connections: u64,
    pub intercepted_requests: u64,
    pub bypass_connections: u64,
    pub passthrough_connections: u64,
    pub forward_errors: u64,
    pub last_intercept_host: Option<String>,
    pub last_intercept_path: Option<String>,
    pub cached_leaf_certs: usize,
    pub ca_fingerprint_sha256: String,
    pub bypass_patterns: Vec<String>,
}

pub struct MitmProxyConfig {
    pub bind_addr: String,
    pub gateway_base_url: String,
    pub bypass_patterns: Vec<String>,
    pub extra_targets: Vec<MitmTarget>,
}

impl Default for MitmProxyConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:20443".into(),
            gateway_base_url: "http://127.0.0.1:20128".into(),
            bypass_patterns: vec![],
            extra_targets: vec![],
        }
    }
}

struct SharedProxyState {
    ca: RwLock<Arc<CertAuthority>>,
    gateway_base_url: RwLock<String>,
    bypass_patterns: RwLock<Vec<String>>,
    extra_targets: RwLock<Vec<MitmTarget>>,
    http_client: reqwest::Client,
    running: AtomicBool,
    bind_addr: RwLock<Option<String>>,
    started_at: RwLock<Option<String>>,
    total_connections: AtomicU64,
    active_connections: AtomicU64,
    intercepted_requests: AtomicU64,
    bypass_connections: AtomicU64,
    passthrough_connections: AtomicU64,
    forward_errors: AtomicU64,
    last_intercept_host: RwLock<Option<String>>,
    last_intercept_path: RwLock<Option<String>>,
}

pub struct MitmProxyHandle {
    state: Arc<SharedProxyState>,
    shutdown_tx: RwLock<Option<oneshot::Sender<()>>>,
}

impl MitmProxyHandle {
    pub async fn start(ca: Arc<CertAuthority>, cfg: MitmProxyConfig) -> Result<Self, String> {
        let listener = TcpListener::bind(&cfg.bind_addr)
            .await
            .map_err(|e| format!("failed to bind MITM proxy on {}: {e}", cfg.bind_addr))?;
        let actual_addr = listener
            .local_addr()
            .map_err(|e| format!("local_addr: {e}"))?
            .to_string();

        let state = Arc::new(SharedProxyState {
            ca: RwLock::new(ca),
            gateway_base_url: RwLock::new(cfg.gateway_base_url),
            bypass_patterns: RwLock::new(cfg.bypass_patterns),
            extra_targets: RwLock::new(cfg.extra_targets),
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .map_err(|e| format!("reqwest client: {e}"))?,
            running: AtomicBool::new(true),
            bind_addr: RwLock::new(Some(actual_addr)),
            started_at: RwLock::new(Some(chrono::Utc::now().to_rfc3339())),
            total_connections: AtomicU64::new(0),
            active_connections: AtomicU64::new(0),
            intercepted_requests: AtomicU64::new(0),
            bypass_connections: AtomicU64::new(0),
            passthrough_connections: AtomicU64::new(0),
            forward_errors: AtomicU64::new(0),
            last_intercept_host: RwLock::new(None),
            last_intercept_path: RwLock::new(None),
        });

        let (tx, mut rx) = oneshot::channel::<()>();
        let loop_state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut rx => {
                        loop_state.running.store(false, Ordering::SeqCst);
                        break;
                    }
                    accepted = listener.accept() => {
                        match accepted {
                            Ok((stream, _peer)) => {
                                let conn_state = loop_state.clone();
                                tokio::spawn(async move {
                                    conn_state.total_connections.fetch_add(1, Ordering::Relaxed);
                                    conn_state.active_connections.fetch_add(1, Ordering::Relaxed);
                                    let _ = handle_client_connection(stream, conn_state.clone()).await;
                                    conn_state.active_connections.fetch_sub(1, Ordering::Relaxed);
                                });
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        });

        Ok(Self {
            state,
            shutdown_tx: RwLock::new(Some(tx)),
        })
    }

    pub fn bind_addr(&self) -> String {
        self.state
            .bind_addr
            .read()
            .unwrap()
            .clone()
            .unwrap_or_default()
    }

    pub fn stats(&self) -> MitmRuntimeStats {
        let ca = self.state.ca.read().unwrap().clone();
        MitmRuntimeStats {
            running: self.state.running.load(Ordering::SeqCst),
            bind_addr: self.state.bind_addr.read().unwrap().clone(),
            gateway_base_url: self.state.gateway_base_url.read().unwrap().clone(),
            started_at: self.state.started_at.read().unwrap().clone(),
            total_connections: self.state.total_connections.load(Ordering::Relaxed),
            active_connections: self.state.active_connections.load(Ordering::Relaxed),
            intercepted_requests: self.state.intercepted_requests.load(Ordering::Relaxed),
            bypass_connections: self.state.bypass_connections.load(Ordering::Relaxed),
            passthrough_connections: self.state.passthrough_connections.load(Ordering::Relaxed),
            forward_errors: self.state.forward_errors.load(Ordering::Relaxed),
            last_intercept_host: self.state.last_intercept_host.read().unwrap().clone(),
            last_intercept_path: self.state.last_intercept_path.read().unwrap().clone(),
            cached_leaf_certs: ca.cached_leaf_count(),
            ca_fingerprint_sha256: ca.ca_fingerprint_sha256(),
            bypass_patterns: self.state.bypass_patterns.read().unwrap().clone(),
        }
    }

    pub fn set_bypass_patterns(&self, patterns: Vec<String>) {
        *self.state.bypass_patterns.write().unwrap() = patterns;
    }

    pub fn replace_ca(&self, new_ca: Arc<CertAuthority>) {
        *self.state.ca.write().unwrap() = new_ca;
    }

    pub fn stop(&self) {
        self.state.running.store(false, Ordering::SeqCst);
        if let Some(tx) = self.shutdown_tx.write().unwrap().take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for MitmProxyHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn resolve_route_with_extras(state: &SharedProxyState, host: &str) -> ConnectionRoute {
    let bypass = state.bypass_patterns.read().unwrap().clone();
    match route_connection(host, &bypass) {
        ConnectionRoute::Bypass { reason } => ConnectionRoute::Bypass { reason },
        ConnectionRoute::Target(t) => ConnectionRoute::Target(t),
        ConnectionRoute::Passthrough => {
            let h = host.to_ascii_lowercase();
            let extras = state.extra_targets.read().unwrap();
            if let Some(t) = extras
                .iter()
                .find(|t| t.hosts.iter().any(|x| x.eq_ignore_ascii_case(&h)))
                .cloned()
            {
                ConnectionRoute::Target(t)
            } else {
                ConnectionRoute::Passthrough
            }
        }
    }
}

async fn handle_client_connection(
    stream: TcpStream,
    state: Arc<SharedProxyState>,
) -> Result<(), String> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    let n = reader
        .read_line(&mut request_line)
        .await
        .map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(());
    }
    let mut parts = request_line.trim().split_whitespace();
    let method = parts.next().unwrap_or("");
    let target_raw = parts.next().unwrap_or("");

    // Drain remaining request headers
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).await.map_err(|e| e.to_string())?;
        if bytes == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }

    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host_port(target_raw, 443);
        let mut client_stream = reader.into_inner();
        let route = resolve_route_with_extras(&state, &host);

        match route {
            ConnectionRoute::Bypass { .. } => {
                state.bypass_connections.fetch_add(1, Ordering::Relaxed);
                tunnel_tcp(client_stream, &host, port).await
            }
            ConnectionRoute::Passthrough => {
                state.passthrough_connections.fetch_add(1, Ordering::Relaxed);
                tunnel_tcp(client_stream, &host, port).await
            }
            ConnectionRoute::Target(target) => {
                client_stream
                    .write_all(b"HTTP/1.1 200 Connection Established\r\nProxy-Agent: omniroute-rs-mitm\r\n\r\n")
                    .await
                    .map_err(|e| e.to_string())?;
                client_stream.flush().await.map_err(|e| e.to_string())?;

                let ca = state.ca.read().unwrap().clone();
                let server_cfg = ca.build_server_config_for_host(&host)?;
                let acceptor = TlsAcceptor::from(server_cfg);
                let tls_stream = acceptor
                    .accept(client_stream)
                    .await
                    .map_err(|e| format!("TLS accept failed for {host}: {e}"))?;

                handle_intercepted_http(tls_stream, &host, port, &target, &state).await
            }
        }
    } else {
        // Plain HTTP forward proxy mode
        let content_len: usize = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; content_len];
        if content_len > 0 {
            reader.read_exact(&mut body).await.map_err(|e| e.to_string())?;
        }
        let mut client_stream = reader.into_inner();
        let parsed_url = reqwest::Url::parse(target_raw).ok();
        let host = parsed_url
            .as_ref()
            .and_then(|u| u.host_str())
            .unwrap_or("")
            .to_string();
        let path = parsed_url
            .as_ref()
            .map(|u| match u.query() {
                Some(q) => format!("{}?{q}", u.path()),
                None => u.path().to_string(),
            })
            .unwrap_or_else(|| target_raw.to_string());

        let target_url = match resolve_route_with_extras(&state, &host) {
            ConnectionRoute::Target(t) if target_matches_path(&t, &path) => {
                state.intercepted_requests.fetch_add(1, Ordering::Relaxed);
                *state.last_intercept_host.write().unwrap() = Some(host.clone());
                *state.last_intercept_path.write().unwrap() = Some(path.clone());
                let gw = state.gateway_base_url.read().unwrap().clone();
                format!("{}{}", gw.trim_end_matches('/'), path)
            }
            _ => target_raw.to_string(),
        };
        forward_http_exchange(&mut client_stream, method, &target_url, &headers, body, &state).await
    }
}

async fn tunnel_tcp(mut client: TcpStream, host: &str, port: u16) -> Result<(), String> {
    let mut upstream = match TcpStream::connect((host, port)).await {
        Ok(s) => s,
        Err(e) => {
            let _ = client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await;
            return Err(format!("connect {host}:{port}: {e}"));
        }
    };
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .map_err(|e| e.to_string())?;
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

async fn handle_intercepted_http<S: AsyncRead + AsyncWrite + Unpin>(
    tls_stream: S,
    host: &str,
    port: u16,
    target: &MitmTarget,
    state: &SharedProxyState,
) -> Result<(), String> {
    let mut reader = BufReader::new(tls_stream);
    let mut req_line = String::new();
    let n = reader.read_line(&mut req_line).await.map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(());
    }
    let mut parts = req_line.trim().split_whitespace();
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).await.map_err(|e| e.to_string())?;
        if bytes == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let content_len: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; content_len];
    if content_len > 0 {
        reader.read_exact(&mut body).await.map_err(|e| e.to_string())?;
    }

    let mut stream = reader.into_inner();
    let forward_url = if target.endpoint_patterns.is_empty() || target_matches_path(target, &path) {
        state.intercepted_requests.fetch_add(1, Ordering::Relaxed);
        *state.last_intercept_host.write().unwrap() = Some(host.to_string());
        *state.last_intercept_path.write().unwrap() = Some(path.clone());
        let gw = state.gateway_base_url.read().unwrap().clone();
        format!("{}{}", gw.trim_end_matches('/'), path)
    } else if port == 443 {
        format!("https://{host}{path}")
    } else {
        format!("https://{host}:{port}{path}")
    };

    forward_http_exchange(&mut stream, &method, &forward_url, &headers, body, state).await
}

async fn forward_http_exchange<S: AsyncWrite + Unpin>(
    client: &mut S,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Vec<u8>,
    state: &SharedProxyState,
) -> Result<(), String> {
    let req_method = reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::POST);
    let mut builder = state.http_client.request(req_method, url);
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("host")
            || k.eq_ignore_ascii_case("content-length")
            || k.eq_ignore_ascii_case("connection")
            || k.eq_ignore_ascii_case("proxy-connection")
        {
            continue;
        }
        builder = builder.header(k.as_str(), v.as_str());
    }
    builder = builder.header("X-OmniRoute-MITM-Intercepted", "1");
    if !body.is_empty() {
        builder = builder.body(body);
    }

    let resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            state.forward_errors.fetch_add(1, Ordering::Relaxed);
            let msg = format!("{{\"error\":{{\"message\":\"MITM forward failed: {e}\"}}}}");
            let raw = format!(
                "HTTP/1.1 502 Bad Gateway\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                msg.len(),
                msg
            );
            let _ = client.write_all(raw.as_bytes()).await;
            let _ = client.flush().await;
            return Err(e.to_string());
        }
    };

    let status = resp.status();
    let resp_headers = resp.headers().clone();
    let resp_bytes = resp.bytes().await.unwrap_or_default();

    let mut head = format!(
        "HTTP/1.1 {} {}\r\n",
        status.as_u16(),
        status.canonical_reason().unwrap_or("OK")
    );
    for (k, v) in &resp_headers {
        let name = k.as_str();
        if name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("content-length")
            || name.eq_ignore_ascii_case("connection")
        {
            continue;
        }
        if let Ok(val) = v.to_str() {
            head.push_str(&format!("{name}: {val}\r\n"));
        }
    }
    head.push_str(&format!(
        "X-OmniRoute-MITM: intercepted\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        resp_bytes.len()
    ));

    client
        .write_all(head.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    client
        .write_all(&resp_bytes)
        .await
        .map_err(|e| e.to_string())?;
    client.flush().await.map_err(|e| e.to_string())?;
    Ok(())
}

fn split_host_port(authority: &str, default_port: u16) -> (String, u16) {
    if let Some((h, p)) = authority.rsplit_once(':') {
        if let Ok(port) = p.parse::<u16>() {
            return (h.trim_matches(['[', ']']).to_string(), port);
        }
    }
    (authority.to_string(), default_port)
}
