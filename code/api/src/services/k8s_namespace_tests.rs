use super::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http::{Request, Response};
use http_body_util::BodyExt;
use kube::client::Body;
use serde_json::json;
use tower::service_fn;

// Each request consumes one step; extra requests fail and unconsumed steps are asserted below.
fn scripted_client(
    steps: Vec<(&'static str, u16)>,
) -> (K8sClient, Arc<Mutex<VecDeque<(&'static str, u16)>>>) {
    let remaining = Arc::new(Mutex::new(VecDeque::from(steps)));
    let steps = remaining.clone();
    let service = service_fn(move |request: Request<Body>| {
        let (method, status) = steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected kube request");
        async move {
            assert_eq!(request.method(), method);
            let namespace = json!({"apiVersion": "v1", "kind": "Namespace", "metadata": {"name": "regression"}});
            if method == "POST" {
                assert_eq!(request.uri().path(), "/api/v1/namespaces");
                let body = request.into_body().collect().await.unwrap().to_bytes();
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                    namespace
                );
            } else {
                assert_eq!(request.uri().path(), "/api/v1/namespaces/regression");
            }
            let body = if status < 400 {
                namespace
            } else {
                json!({"apiVersion": "v1", "kind": "Status", "status": "Failure", "code": status,
                    "reason": "InjectedError", "message": format!("injected {status}")})
            };
            Ok::<_, std::convert::Infallible>(
                Response::builder()
                    .status(status)
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
        }
    });
    (
        K8sClient::from_client(Client::new(service, "default")),
        remaining,
    )
}

#[tokio::test(start_paused = true)]
async fn ensure_namespace_existing_only_gets() {
    let (client, remaining) = scripted_client(vec![("GET", 200)]);
    let start = tokio::time::Instant::now();
    client.ensure_namespace("regression").await.unwrap();
    assert!(remaining.lock().unwrap().is_empty());
    assert_eq!(start.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn ensure_namespace_create_polls_until_visible_including_conflict() {
    for create_status in [201, 409] {
        let (client, remaining) = scripted_client(vec![
            ("GET", 404),
            ("POST", create_status),
            ("GET", 404),
            ("GET", 200),
        ]);
        let start = tokio::time::Instant::now();
        client.ensure_namespace("regression").await.unwrap();
        assert!(remaining.lock().unwrap().is_empty());
        assert_eq!(start.elapsed(), Duration::from_millis(100));
    }
}

#[tokio::test(start_paused = true)]
async fn ensure_namespace_api_errors_stop_at_the_failing_stage() {
    for status in [403, 500] {
        for (steps, context) in [
            (vec![("GET", status)], "Failed to get namespace"),
            (
                vec![("GET", 404), ("POST", status)],
                "Failed to create namespace",
            ),
            (
                vec![("GET", 404), ("POST", 201), ("GET", status)],
                "Failed to verify namespace",
            ),
        ] {
            let (client, remaining) = scripted_client(steps);
            let start = tokio::time::Instant::now();
            let error = client
                .ensure_namespace("regression")
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(context), "{error}");
            assert!(error.contains(&format!("injected {status}")), "{error}");
            assert!(remaining.lock().unwrap().is_empty());
            assert_eq!(start.elapsed(), Duration::ZERO);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn ensure_namespace_transport_error_is_not_absence() {
    let service = service_fn(|request: Request<Body>| async move {
        assert_eq!(request.method(), "GET");
        Err::<Response<Body>, _>(std::io::Error::other("injected transport failure"))
    });
    let client = K8sClient::from_client(Client::new(service, "default"));
    let error = client
        .ensure_namespace("regression")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("Failed to get namespace"), "{error}");
}

#[tokio::test(start_paused = true)]
async fn ensure_namespace_visibility_timeout_is_bounded() {
    let mut steps = vec![("GET", 404), ("POST", 201)];
    steps.extend(std::iter::repeat_n(("GET", 404), 50));
    let (client, remaining) = scripted_client(steps);
    let start = tokio::time::Instant::now();
    let error = tokio::time::timeout(
        Duration::from_secs(6),
        client.ensure_namespace("regression"),
    )
    .await
    .expect("visibility polling must terminate")
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("Namespace 'regression' was not visible after creation"),
        "{error}"
    );
    assert!(remaining.lock().unwrap().is_empty());
    assert_eq!(start.elapsed(), Duration::from_secs(5));
}
