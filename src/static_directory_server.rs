use axum::extract::{Path as ExtractPath, Request};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect};
use axum::routing::{any, get_service};
use reqwest::{header, StatusCode, Url};
use std::net::SocketAddr;
use std::path::Path;
use tokio::sync::oneshot;
use tower_http::services::ServeDir;

/// A convenient async HTTP server that serves the content of a folder. The server only listens to
/// `127.0.0.1` and uses a random port. This makes it safe to run multiple instances. Its perfect to
/// use for testing HTTP file requests.
pub struct StaticDirectoryServer {
    local_addr: SocketAddr,
    shutdown_sender: Option<oneshot::Sender<()>>,
}

impl StaticDirectoryServer {
    /// Returns the root `Url` to the server.
    pub fn url(&self) -> Url {
        Url::parse(&format!("http://localhost:{}", self.local_addr.port())).unwrap()
    }
}

impl StaticDirectoryServer {
    pub async fn new(path: impl AsRef<Path>) -> Result<Self, StaticDirectoryServerError> {
        let service = get_service(ServeDir::new(path));

        // Create a router that will serve the static files
        let app = axum::Router::new()
            .nest_service("/", service)
            .route(
                "/redirect-method",
                any(StaticDirectoryServer::redirect_method),
            )
            .route(
                "/only-works-with-method/:method",
                any(StaticDirectoryServer::method_matcher),
            );

        // Construct the server that will listen on localhost but with a *random port*. The random
        // port is very important because it enables creating multiple instances at the same time.
        // We need this to be able to run tests in parallel.
        let addr = SocketAddr::new([127, 0, 0, 1].into(), 0);
        let listener = tokio::net::TcpListener::bind(addr).await?;

        // Get the address of the server so we can bind to it at a later stage.
        let addr = listener.local_addr()?;

        // Setup a graceful shutdown trigger which is fired when this instance is dropped.
        let (tx, rx) = oneshot::channel();

        // Spawn the server in the background.
        tokio::spawn(async move {
            let _ = axum::serve(listener, app.into_make_service())
                .with_graceful_shutdown(async {
                    rx.await.ok();
                })
                .await;
        });

        Ok(Self {
            local_addr: addr,
            shutdown_sender: Some(tx),
        })
    }

    // Redirects to a URL that contains the HTTP method used in the request.
    async fn redirect_method<B>(r: Request<B>) -> Redirect {
        let method = r.method().to_string();
        let destination = format!("/only-works-with-method/{}", method);
        Redirect::temporary(&destination)
    }

    // Responds with 206 Partial Content if the method matches the one in the path, otherwise
    // 405 Method Not Allowed
    async fn method_matcher<B>(
        ExtractPath(method): ExtractPath<String>,
        r: Request<B>,
    ) -> impl IntoResponse {
        let mut headers = HeaderMap::new();
        if method != r.method().as_str() {
            let empty: &[u8] = &[];
            return (StatusCode::METHOD_NOT_ALLOWED, headers, empty);
        }

        headers.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
        headers.insert(
            header::CONTENT_TYPE,
            "application/octet-stream".parse().unwrap(),
        );
        let bytes: &[u8] = &[1, 2, 3, 4, 5, 6, 7, 8, 9];
        headers.insert(header::CONTENT_LENGTH, bytes.len().into());
        (StatusCode::PARTIAL_CONTENT, headers, bytes)
    }
}

impl Drop for StaticDirectoryServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_sender.take() {
            let _ = tx.send(());
        }
    }
}
/// Error type used for [`StaticDirectoryServerError`]
#[derive(Debug, thiserror::Error)]
pub enum StaticDirectoryServerError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
