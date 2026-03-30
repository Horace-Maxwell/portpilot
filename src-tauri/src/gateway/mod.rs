use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::{Path as AxumPath, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::Response;
use axum::routing::any;
use axum::Router;
use portpicker::is_free_tcp;
use reqwest::Client;

use crate::storage::store::ProjectStore;

#[derive(Clone)]
struct GatewayState {
    client: Client,
    store: Arc<ProjectStore>,
}

pub async fn start_gateway(store: Arc<ProjectStore>) -> Result<u16, String> {
    let port = choose_gateway_port(42300).ok_or_else(|| "Could not find a free gateway port.".to_string())?;
    let state = GatewayState {
        client: Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| error.to_string())?,
        store,
    };

    let app = build_router(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    tauri::async_runtime::spawn(async move {
        if let Ok(listener) = tokio::net::TcpListener::bind(addr).await {
            let _ = axum::serve(listener, app).await;
        }
    });

    Ok(port)
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
        return response_text(StatusCode::NOT_FOUND, "No PortPilot route matched this host.");
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
        return response_text(StatusCode::INTERNAL_SERVER_ERROR, "PortPilot could not read the project registry.");
    };

    let Some(project) = projects.into_iter().find(|item| item.slug == slug) else {
        return response_text(StatusCode::NOT_FOUND, "Unknown PortPilot route.");
    };

    let Some(port) = project.resolved_port.or(project.preferred_port) else {
        return response_text(StatusCode::SERVICE_UNAVAILABLE, "Project does not have an active target port yet.");
    };

    let path = if rest.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", rest)
    };

    let query = request.uri().query().map(|value| format!("?{value}")).unwrap_or_default();
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
        .unwrap_or_else(|_| response_text(StatusCode::BAD_GATEWAY, "Gateway failed to build the response body."))
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use reqwest::Client;
    use uuid::Uuid;

    use super::{build_router, GatewayState};
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
}
