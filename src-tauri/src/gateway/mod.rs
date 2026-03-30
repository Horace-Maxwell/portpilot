use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::{Path as AxumPath, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::Response;
use axum::routing::any;
use axum::Router;
use portpicker::is_free_tcp;
use reqwest::Client;

use crate::core::models::{LocalHttpsCertificateState, LocalHttpsStatus};
use crate::storage::store::ProjectStore;

#[derive(Clone)]
struct GatewayState {
    client: Client,
    store: Arc<ProjectStore>,
}

struct HttpsAssets {
    provider: String,
    certificate_state: LocalHttpsCertificateState,
    cert_path: PathBuf,
    key_path: PathBuf,
    detail: Option<String>,
}

pub async fn start_gateway(
    store: Arc<ProjectStore>,
    data_dir: PathBuf,
) -> Result<(u16, LocalHttpsStatus), String> {
    let http_port = choose_gateway_port(42300)
        .ok_or_else(|| "Could not find a free gateway port.".to_string())?;
    let state = GatewayState {
        client: Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| error.to_string())?,
        store,
    };

    let http_app = build_router(state.clone());
    let addr = SocketAddr::from(([127, 0, 0, 1], http_port));
    tauri::async_runtime::spawn(async move {
        if let Ok(listener) = tokio::net::TcpListener::bind(addr).await {
            let _ = axum::serve(listener, http_app).await;
        }
    });

    let https_status = start_https_gateway(state, http_port, data_dir).await;

    Ok((http_port, https_status))
}

async fn start_https_gateway(
    state: GatewayState,
    http_port: u16,
    data_dir: PathBuf,
) -> LocalHttpsStatus {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let https_port = match choose_gateway_port(http_port.saturating_add(1)) {
        Some(port) => port,
        None => {
            return LocalHttpsStatus {
                enabled: false,
                http_port,
                https_port: None,
                provider: None,
                certificate_state: LocalHttpsCertificateState::Missing,
                detail: Some(
                    "PortPilot could not find a free localhost port for the HTTPS gateway."
                        .to_string(),
                ),
            }
        }
    };

    let assets = match prepare_https_assets(&data_dir) {
        Ok(Some(assets)) => assets,
        Ok(None) => {
            return LocalHttpsStatus {
                enabled: false,
                http_port,
                https_port: None,
                provider: None,
                certificate_state: LocalHttpsCertificateState::Missing,
                detail: Some(
                    "PortPilot could not find mkcert or openssl, so HTTPS is currently unavailable."
                        .to_string(),
                ),
            }
        }
        Err(error) => {
            return LocalHttpsStatus {
                enabled: false,
                http_port,
                https_port: None,
                provider: None,
                certificate_state: LocalHttpsCertificateState::Error,
                detail: Some(error),
            }
        }
    };

    let config =
        match axum_server::tls_rustls::RustlsConfig::from_pem_file(&assets.cert_path, &assets.key_path)
            .await
        {
            Ok(config) => config,
            Err(error) => {
                return LocalHttpsStatus {
                    enabled: false,
                    http_port,
                    https_port: None,
                    provider: Some(assets.provider),
                    certificate_state: LocalHttpsCertificateState::Error,
                    detail: Some(format!(
                        "PortPilot generated local HTTPS certificates, but Rustls could not load them: {error}"
                    )),
                }
            }
        };

    let https_app = build_router(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], https_port));
    tauri::async_runtime::spawn(async move {
        let _ = axum_server::bind_rustls(addr, config)
            .serve(https_app.into_make_service())
            .await;
    });

    LocalHttpsStatus {
        enabled: true,
        http_port,
        https_port: Some(https_port),
        provider: Some(assets.provider),
        certificate_state: assets.certificate_state,
        detail: assets.detail,
    }
}

fn build_router(state: GatewayState) -> Router {
    Router::new()
        .route("/", any(proxy_root))
        .route("/p/{slug}", any(proxy_project_root))
        .route("/p/{slug}/{*rest}", any(proxy_project_rest))
        .fallback(any(proxy_host))
        .with_state(state)
}

async fn proxy_root() -> Response<Body> {
    response_text(
        StatusCode::OK,
        "PortPilot gateway is running. Open /p/<slug>/ or <slug>.localhost.",
    )
}

async fn proxy_project_root(
    State(state): State<GatewayState>,
    AxumPath(slug): AxumPath<String>,
    method: Method,
    headers: HeaderMap,
    request: Request,
) -> Response<Body> {
    proxy_to_slug(state, method, headers, request, &slug, "").await
}

async fn proxy_project_rest(
    State(state): State<GatewayState>,
    AxumPath((slug, rest)): AxumPath<(String, String)>,
    method: Method,
    headers: HeaderMap,
    request: Request,
) -> Response<Body> {
    proxy_to_slug(state, method, headers, request, &slug, &rest).await
}

async fn proxy_host(
    State(state): State<GatewayState>,
    method: Method,
    headers: HeaderMap,
    request: Request,
) -> Response<Body> {
    let host = headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let slug = host
        .split(':')
        .next()
        .unwrap_or_default()
        .strip_suffix(".localhost")
        .unwrap_or_default()
        .to_string();

    if slug.is_empty() || slug == "gateway" {
        return response_text(
            StatusCode::NOT_FOUND,
            "No PortPilot route matched this host.",
        );
    }

    proxy_to_slug(state, method, headers, request, &slug, "").await
}

async fn proxy_to_slug(
    state: GatewayState,
    method: Method,
    headers: HeaderMap,
    request: Request,
    slug: &str,
    rest: &str,
) -> Response<Body> {
    let Ok(projects) = state.store.list() else {
        return response_text(
            StatusCode::INTERNAL_SERVER_ERROR,
            "PortPilot could not read the project registry.",
        );
    };

    let Some(project) = projects.into_iter().find(|item| item.slug == slug) else {
        return response_text(StatusCode::NOT_FOUND, "Unknown PortPilot route.");
    };

    let Some(port) = project.resolved_port.or(project.preferred_port) else {
        return response_text(
            StatusCode::SERVICE_UNAVAILABLE,
            "Project does not have an active target port yet.",
        );
    };

    let path = if rest.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", rest)
    };

    let query = request
        .uri()
        .query()
        .map(|value| format!("?{value}"))
        .unwrap_or_default();
    let target_url = format!("http://127.0.0.1:{port}{path}{query}");
    let body = match to_bytes(request.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(_) => return response_text(StatusCode::BAD_REQUEST, "Could not read request body."),
    };

    let mut upstream = state.client.request(method, &target_url);
    for (name, value) in headers.iter() {
        if name.as_str().eq_ignore_ascii_case("host") {
            continue;
        }
        upstream = upstream.header(name, value);
    }

    let response = match upstream.body(body).send().await {
        Ok(response) => response,
        Err(error) => {
            return response_text(
                StatusCode::BAD_GATEWAY,
                &format!("Could not reach the managed app on port {port}: {error}"),
            )
        }
    };

    let status = response.status();
    let response_headers = response.headers().clone();
    let response_body = match response.bytes().await {
        Ok(body) => body,
        Err(error) => {
            return response_text(
                StatusCode::BAD_GATEWAY,
                &format!("Could not read the managed app response: {error}"),
            )
        }
    };

    let mut outgoing = Response::builder().status(status);
    for (name, value) in response_headers.iter() {
        if let (Ok(header_name), Ok(header_value)) = (
            HeaderName::from_bytes(name.as_ref()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            outgoing = outgoing.header(header_name, header_value);
        }
    }
    outgoing
        .body(Body::from(response_body))
        .unwrap_or_else(|_| {
            response_text(
                StatusCode::BAD_GATEWAY,
                "Gateway failed to build the response body.",
            )
        })
}

fn response_text(status: StatusCode, message: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(Body::from(message.to_string()))
        .expect("valid plain text response")
}

fn choose_gateway_port(start: u16) -> Option<u16> {
    (start..=start + 20).find(|port| is_free_tcp(*port))
}

fn prepare_https_assets(data_dir: &Path) -> Result<Option<HttpsAssets>, String> {
    let tls_dir = data_dir.join("gateway").join("tls");
    fs::create_dir_all(&tls_dir).map_err(|error| error.to_string())?;

    let cert_path = tls_dir.join("localhost-cert.pem");
    let key_path = tls_dir.join("localhost-key.pem");

    if command_exists("mkcert") {
        let output = Command::new("mkcert")
            .args([
                "-cert-file",
                cert_path.to_string_lossy().as_ref(),
                "-key-file",
                key_path.to_string_lossy().as_ref(),
                "localhost",
                "gateway.localhost",
                "*.localhost",
                "127.0.0.1",
            ])
            .output()
            .map_err(|error| format!("Failed to launch mkcert: {error}"))?;
        if output.status.success() {
            return Ok(Some(HttpsAssets {
                provider: "mkcert".to_string(),
                certificate_state: LocalHttpsCertificateState::Trusted,
                cert_path,
                key_path,
                detail: Some(
                    "PortPilot generated a trusted localhost certificate with mkcert."
                        .to_string(),
                ),
            }));
        }
    }

    if command_exists("openssl") {
        let output = Command::new("openssl")
            .args([
                "req",
                "-x509",
                "-nodes",
                "-newkey",
                "rsa:2048",
                "-keyout",
                key_path.to_string_lossy().as_ref(),
                "-out",
                cert_path.to_string_lossy().as_ref(),
                "-sha256",
                "-days",
                "365",
                "-subj",
                "/CN=gateway.localhost",
                "-addext",
                "subjectAltName=DNS:localhost,DNS:gateway.localhost,DNS:*.localhost,IP:127.0.0.1",
            ])
            .output()
            .map_err(|error| format!("Failed to launch openssl: {error}"))?;
        if output.status.success() {
            return Ok(Some(HttpsAssets {
                provider: "openssl".to_string(),
                certificate_state: LocalHttpsCertificateState::NeedsTrust,
                cert_path,
                key_path,
                detail: Some(
                    "PortPilot generated a self-signed localhost certificate. Install mkcert if you want the browser to trust HTTPS automatically."
                        .to_string(),
                ),
            }));
        }
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    Ok(None)
}

fn command_exists(binary: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&paths).any(|path| path.join(binary).is_file())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use reqwest::Client;
    use uuid::Uuid;

    use super::{build_router, choose_gateway_port, GatewayState};
    use crate::storage::store::ProjectStore;

    #[test]
    fn builds_gateway_router_without_panicking() {
        let temp_dir = std::env::temp_dir().join(format!("portpilot-gateway-{}", Uuid::new_v4()));
        let store = Arc::new(ProjectStore::load(temp_dir.join("store.sqlite")).unwrap());
        let state = GatewayState {
            client: Client::new(),
            store,
        };

        let router = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build_router(state)));
        assert!(router.is_ok());

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn chooses_gateway_port_from_requested_range() {
        let port = choose_gateway_port(42500);
        assert!(port.is_some());
    }
}
