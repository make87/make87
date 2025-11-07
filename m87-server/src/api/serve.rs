use axum::{
    http::{header, Method},
    response::IntoResponse,
    routing::get,
    Router,
};
use futures::StreamExt;
use m87_shared::{forward::ForwardAccess, roles::Role};
use mongodb::bson::{doc, oid::ObjectId};
use rustls::{
    crypto::ring::default_provider,
    pki_types::{CertificateDer, PrivateKeyDer},
};
use rustls::{pki_types::PrivatePkcs8KeyDer, ServerConfig};
use std::{sync::Arc, time::Duration};
use tokio::io::{self, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio_rustls::{server::TlsStream, TlsAcceptor};
use tokio_rustls_acme::{caches::DirCache, AcmeConfig};
use tokio_stream::wrappers::TcpListenerStream;
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowOrigin, CorsLayer},
    sensitive_headers::SetSensitiveHeadersLayer,
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tracing::{info, warn};

use crate::{
    api::{auth, device},
    auth::claims::Claims,
    config::AppConfig,
    db::Mongo,
    models::device::DeviceDoc,
    relay::relay_state::RelayState,
    response::{ServerError, ServerResult},
    util::{app_state::AppState, tcp_proxy::proxy_bidirectional},
};
use rcgen::generate_simple_self_signed;
use tokio_yamux::{Config as YamuxConfig, Session};

async fn get_status() -> impl IntoResponse {
    "ok".to_string()
}

pub async fn serve(
    db: Arc<Mongo>,
    relay: Arc<RelayState>,
    cfg: Arc<AppConfig>,
) -> ServerResult<()> {
    // Ensure rustls has a crypto provider before anything touches TLS
    rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider())
        .expect("failed to install ring crypto provider");

    let state = AppState {
        db: db.clone(),
        config: cfg.clone(),
        relay: relay.clone(),
    };

    // ===== REST on loopback =====
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::any())
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::HeaderName::from_static("sec-websocket-protocol"),
        ]);

    let app = Router::new()
        .nest("/auth", auth::create_route())
        .nest("/device", device::create_route())
        .route("/status", get(get_status))
        .with_state(state.clone())
        .layer(cors)
        .layer(SetSensitiveHeadersLayer::new(std::iter::once(
            header::AUTHORIZATION,
        )))
        .layer(TimeoutLayer::new(Duration::from_secs(30)))
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new());

    let rest_listener = TcpListener::bind(("127.0.0.1", cfg.rest_port))
        .await
        .expect("bind REST");
    info!("REST listening on 127.0.0.1:{}", cfg.rest_port);

    let rest_task = tokio::spawn(async move {
        if let Err(e) = axum::serve(rest_listener, app).await {
            warn!("Axum server failed: {e:?}");
        }
    });

    // === TLS (ACME or self-signed) ===
    // Don't spawn here — serve_tls_or_selfsigned already does internal spawns.
    serve_tls_or_selfsigned(cfg.clone(), state.clone(), relay.clone()).await?;

    // === Wait for REST task forever ===
    let _ = rest_task.await;
    Ok(())
}

async fn serve_tls_or_selfsigned(
    cfg: Arc<AppConfig>,
    state: AppState,
    relay: Arc<RelayState>,
) -> ServerResult<()> {
    let cache_dir = "/app/certs";
    let public = cfg.public_address.clone();
    let control = format!("control.{public}");
    let tcp = TcpListener::bind(("0.0.0.0", cfg.unified_port))
        .await
        .expect("bind TLS");
    let incoming = TcpListenerStream::new(tcp);

    if cfg.is_staging {
        // === Self-signed localhost mode ===
        let ck = generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])
            .map_err(|err| ServerError::internal_error(&format!("{}", err,)))?;

        // 2) DER forms for rustls
        let cert_der: CertificateDer<'static> = ck.cert.der().clone().into();

        // rcgen 0.14.5 produces PKCS#8; wrap it properly for rustls 0.23:
        let key_bytes = ck.signing_key.serialize_der(); // Vec<u8> (PKCS#8)
        let key_der: PrivateKeyDer<'static> =
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key_bytes));

        // 3) rustls 0.23 builder: provider + protocol versions -> then no client auth
        let provider = Arc::new(default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
            .map_err(|err| ServerError::internal_error(&format!("{}", err,)))?
            .with_no_client_auth()
            .with_single_cert(vec![cert_der], key_der)
            .map_err(|err| ServerError::internal_error(&format!("{}", err,)))?;
        let acceptor = TlsAcceptor::from(Arc::new(config));

        tokio::spawn(async move {
            let mut incoming = incoming;
            while let Some(Ok(stream)) = incoming.next().await {
                let accept = acceptor.accept(stream);
                let state = state.clone();
                let relay = relay.clone();
                tokio::spawn(async move {
                    match accept.await {
                        Ok(mut tls) => {
                            let sni = tls
                                .get_ref()
                                .1
                                .server_name()
                                .unwrap_or("localhost")
                                .to_string();
                            let _ = handle_sni(&sni, tls, &state).await;
                        }
                        Err(e) => tracing::warn!("TLS handshake failed: {e:?}"),
                    }
                });
            }
        });
    } else {
        // === ACME for public domain ===
        let mut tls_incoming = AcmeConfig::new([public.as_str(), control.as_str()])
            .contact_push(format!("mailto:{}", state.config.cert_contact))
            .cache(DirCache::new(cache_dir))
            .directory_lets_encrypt(!cfg.is_staging)
            .incoming(incoming, Vec::new());

        tokio::spawn(async move {
            while let Some(conn) = tls_incoming.next().await {
                match conn {
                    Ok(mut tls) => {
                        let sni = tls.get_ref().1.server_name().unwrap_or("").to_string();
                        tracing::info!("ACME TLS SNI: {}", sni);
                        let _ = handle_sni(&sni, tls, &state).await;
                    }
                    Err(e) => tracing::warn!("ACME/TLS accept error: {e:?}"),
                }
            }
        });
    }
    Ok(())
}

async fn handle_sni(sni: &str, mut tls: TlsStream<tokio::net::TcpStream>, state: &AppState) {
    if sni.is_empty() {
        warn!("TLS no SNI; closing");
        let _ = tls.shutdown().await;
        return;
    }

    let public = &state.config.public_address;
    let control_host = format!("control.{public}");

    // === REST ===
    if sni == *public {
        if let Err(e) = proxy_to_rest(&mut tls, state.config.rest_port).await {
            warn!("REST proxy failed: {e:?}");
        }
        return;
    }

    // === CONTROL ===
    if sni == control_host {
        if let Err(e) =
            handle_control_tunnel(state.relay.clone(), tls, &state.config.forward_secret).await
        {
            warn!("control tunnel failed: {e:?}");
        }
        return;
    }

    // === DEVICE or FORWARD ===
    if let Some(prefix) = sni.strip_suffix(public) {
        // e.g. "myapp.device123." -> "myapp.device123."
        let prefix = prefix.trim_end_matches('.');

        let parts: Vec<&str> = prefix.split('.').collect();
        match parts.len() {
            1 => {
                // device123.public_address
                let node_short_id = parts[0];
                if let Err(e) = proxy_to_device_rest(&mut tls, node_short_id, state).await {
                    warn!("device proxy failed: {e:?}");
                }
            }
            n if n >= 2 => {
                // myapp.device123.public_address → forward connection
                if let Err(e) = handle_forward_connection(
                    state.relay.clone(),
                    state.db.clone(),
                    state.config.clone(),
                    sni.to_string(),
                    tls,
                )
                .await
                {
                    warn!("forward failed: {e:?}");
                }
            }
            _ => {
                warn!("invalid SNI format: {}", sni);
                let _ = tls.shutdown().await;
            }
        }
        return;
    }

    // === Fallback ===
    warn!("unmatched SNI: {}", sni);
    let _ = tls.shutdown().await;
}

// --- Helper: extract "Authorization: Bearer <token>" from raw headers ---
fn extract_bearer_token(request: &str) -> Option<String> {
    // 1. Regular Authorization header
    for line in request.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("authorization: bearer ") {
            return line
                .split_once("Bearer ")
                .map(|(_, val)| val.trim().to_string());
        }

        // 2. WebSocket subprotocol form: Sec-WebSocket-Protocol: bearer.<token>
        if lower.starts_with("sec-websocket-protocol: bearer.") {
            // skip past prefix
            let token = &line["Sec-WebSocket-Protocol: bearer.".len()..];
            // strip trailing commas / whitespace
            let token = token
                .split(|c| c == ',' || c == '\r' || c == '\n')
                .next()
                .unwrap_or("")
                .trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }

    None
}

pub async fn proxy_to_device_rest(
    inbound: &mut TlsStream<tokio::net::TcpStream>,
    short_id: &str,
    state: &AppState,
) -> ServerResult<()> {
    // --- 1. Read initial request chunk (headers, maybe some body) ---
    let mut buf = [0u8; 8192];
    let n = inbound.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }
    let request = String::from_utf8_lossy(&buf[..n]);

    // --- 2. Extract and validate token ---
    let token = extract_bearer_token(&request);
    if token.is_none() {
        inbound
            .get_mut()
            .0
            .write_all(b"HTTP/1.1 401 Unauthorized\r\n\r\n")
            .await?;
        return Ok(());
    }
    let claims = Claims::from_bearer_or_key(&token.unwrap(), &state.db, &state.config).await;
    let device = match claims {
        Ok(claims) => claims
            .find_one_with_scope_and_role::<DeviceDoc>(
                &state.db.devices(),
                doc! { "short_id": short_id },
                Role::Editor,
            )
            .await?
            .ok_or_else(|| ServerError::not_found("Device not found"))?,
        Err(_) => {
            inbound
                .get_mut()
                .0
                .write_all(b"HTTP/1.1 401 Unauthorized\r\n\r\n")
                .await?;
            return Ok(());
        }
    };

    let device_id = device.id.clone().unwrap().to_string();

    // --- 3. Find the active tunnel for the node ---
    let Some(conn_arc) = state.relay.get_tunnel(&device_id).await else {
        warn!("No active tunnel for {short_id}");
        inbound
            .get_mut()
            .0
            .write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n")
            .await?;
        return Ok(());
    };

    // --- 4. Open a yamux substream ---
    let mut sess = conn_arc.lock().await;
    let mut sub = match sess.open_stream() {
        Ok(s) => s,
        Err(_) => {
            inbound
                .get_mut()
                .0
                .write_all(b"HTTP/1.1 500 Internal Server Error\r\n\r\n")
                .await?;
            return Ok(());
        }
    };

    // --- 5. Send REST port info to the node (e.g. 80 or configurable) ---
    let rest_port = device.config.server_port;
    sub.write_all(format!("{rest_port}\n").as_bytes()).await?;

    // --- 6. Send already-read request data to the node ---
    sub.write_all(&buf[..n]).await?;

    // --- 7. Start full duplex proxy ---
    tokio::io::copy_bidirectional(inbound, &mut sub).await?;
    Ok(())
}

async fn proxy_to_rest(
    inbound: &mut TlsStream<tokio::net::TcpStream>,
    rest_port: u16,
) -> io::Result<()> {
    let mut outbound = tokio::net::TcpStream::connect(("127.0.0.1", rest_port)).await?;
    let _ = proxy_bidirectional(inbound, &mut outbound).await;
    Ok(())
}

pub async fn handle_control_tunnel(
    relay: Arc<RelayState>,
    tls: TlsStream<tokio::net::TcpStream>,
    secret: &str,
) -> io::Result<()> {
    use tokio::io::AsyncBufReadExt;
    let mut reader = BufReader::new(tls);

    // Expect: "M87 device_id=<id> token=<base64>\n"
    let mut line = String::new();
    if reader.read_line(&mut line).await? == 0 {
        warn!("control: empty handshake");
        return Ok(());
    }
    let device_id = extract_kv(&line, "device_id").unwrap_or_default();
    let token = extract_kv(&line, "token").unwrap_or_default();
    if device_id.is_empty() || token.is_empty() {
        warn!("control: missing device_id/token");
        return Ok(());
    }

    match crate::auth::tunnel_token::verify_tunnel_token(&token, secret) {
        Ok(id_ok) if id_ok == device_id => {}
        Ok(id_ok) => {
            warn!(
                "control: token mismatch got {} but expected {}",
                device_id, id_ok
            );
            return Ok(());
        }
        Err(err) => {
            // print error message
            warn!("control: token invalid {}", err);
            return Ok(());
        }
    }

    {
        let mut tunnels = relay.tunnels.write().await;
        tunnels.remove(&device_id);
    }

    // Upgrade to Yamux
    let base = reader.into_inner();
    let sess = Session::new_server(base, YamuxConfig::default());
    relay.register_tunnel(device_id.clone(), sess).await;
    info!(%device_id, "control tunnel active");
    Ok(())
}

async fn handle_forward_connection(
    relay: Arc<RelayState>,
    db: Arc<Mongo>,
    config: Arc<AppConfig>,
    host: String,
    mut inbound: TlsStream<tokio::net::TcpStream>,
) -> ServerResult<()> {
    let subdomain = host.split('.').next().unwrap_or_default();

    // Lookup forward entry
    let forward_doc = db
        .forwards()
        .find_one(doc! { "device_short_id": subdomain })
        .await?
        .ok_or_else(|| ServerError::not_found("no matching forward"))?;

    // Enforce access policy
    match &forward_doc.access {
        ForwardAccess::Open => {
            // Nothing to check
        }

        ForwardAccess::IpWhitelist(whitelist) => {
            if let Ok(peer) = inbound.get_ref().0.peer_addr() {
                let ip = peer.ip().to_string();
                if !whitelist.iter().any(|a| a == &ip) {
                    warn!(%host, %ip, "blocked by IP whitelist");
                    let _ = inbound.get_mut().0.shutdown().await;
                    return Ok(());
                }
            }
        }
    }

    // Now find tunnel and forward
    let Some(conn_arc) = relay.get_tunnel(&forward_doc.device_id.to_string()).await else {
        warn!(%host, device_id=%forward_doc.device_id, "tunnel not active");
        let _ = inbound.shutdown().await;
        return Ok(());
    };

    let mut sess = conn_arc.lock().await;
    let mut sub = sess
        .open_stream()
        .map_err(|_| ServerError::internal_error("yamux open_stream failed"))?;

    // Send port header to node
    sub.write_all(format!("{}\n", forward_doc.target_port).as_bytes())
        .await?;

    // Forward already-peeked data so the request isn’t truncated
    let mut tmp = [0u8; 1024];
    let n = inbound.read(&mut tmp).await?;
    sub.write_all(&tmp[..n]).await?;

    tokio::spawn(async move {
        let _ = proxy_bidirectional(&mut inbound, &mut sub).await;
    });

    Ok(())
}

fn extract_kv(line: &str, key: &str) -> Option<String> {
    line.split_whitespace().find_map(|part| {
        part.strip_prefix(&(key.to_owned() + "="))
            .map(|s| s.to_string())
    })
}
