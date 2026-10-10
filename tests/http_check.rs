//! Integration tests for the HTTP check against a local server.

use std::time::{Duration, Instant};

use axum::{http::StatusCode, routing::get, Router};
use healthcheck::checks::{ServiceHttp, State};
use tokio::net::TcpListener;

async fn spawn_server() -> String {
    let app = Router::new()
        .route("/ok", get(|| async { StatusCode::OK }))
        .route("/teapot", get(|| async { StatusCode::IM_A_TEAPOT }))
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                StatusCode::OK
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{}", addr)
}

fn http(url: String, timeout_ms: Option<u64>) -> ServiceHttp {
    ServiceHttp {
        url,
        expected_status: None,
        timeout_ms,
    }
}

#[tokio::test]
async fn expected_status_matches() {
    let base = spawn_server().await;
    assert_eq!(
        http(format!("{base}/ok"), None).check().await,
        State::Success
    );

    let state = http(format!("{base}/teapot"), None).check().await;
    assert_eq!(state, State::Failure("Unexpected status: 418".into()));
}

#[tokio::test]
async fn slow_response_hits_timeout() {
    let base = spawn_server().await;
    let started = Instant::now();
    let state = http(format!("{base}/slow"), Some(300)).check().await;
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "timeout not enforced"
    );
    match state {
        State::Failure(reason) => assert!(reason.starts_with("Timeout after 300ms"), "{reason}"),
        other => panic!("expected timeout failure, got {other:?}"),
    }
}

#[tokio::test]
async fn connection_refused_reports_cause() {
    // Bind and immediately drop to get a port nobody listens on.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let state = http(format!("http://127.0.0.1:{port}/"), Some(2000))
        .check()
        .await;
    match state {
        State::Failure(reason) => {
            assert!(reason.starts_with("Request failed: "), "{reason}");
            assert!(
                reason.to_lowercase().contains("refused"),
                "cause chain missing: {reason}"
            );
        }
        other => panic!("expected failure, got {other:?}"),
    }
}

#[tokio::test]
async fn every_check_opens_a_fresh_connection() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Keep-alive server that never closes: a pooled client would reuse the socket.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                while let Ok(n) = sock.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let _ = sock
                        .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                        .await;
                }
            });
        }
    });

    let check = http(format!("http://{addr}/"), Some(2000));
    assert_eq!(check.check().await, State::Success);
    assert_eq!(check.check().await, State::Success);
    assert_eq!(accepted.load(Ordering::SeqCst), 2);
}
