//! Guards the API's RED metrics, and above all their *cardinality*.
//!
//! Two of the three labels on `heraldq_http_requests_total` are derived from
//! caller-controlled input, and both would be unbounded if taken raw:
//!
//! - `path` — swap `MatchedPath` for `uri().path()` and every webhook ULID
//!   becomes its own time series.
//! - `method` — `http::Method` is not an enum of the seven verbs; it has an
//!   `ExtensionInline` variant accepting any HTTP token, so `.to_string()` mints
//!   a series per verb a scanner invents.
//!
//! Both regressions are silent: the code compiles, the metric exports, and
//! nothing breaks until the scrape target melts in production. These tests are
//! the tripwire.

use axum::{
    Router,
    body::Body,
    http::{Method, Request},
    middleware,
    routing::get,
};
use metrics_util::debugging::{DebugValue, DebuggingRecorder};
use sliq::telemetry::track_metrics;
use tower::ServiceExt;

/// Drive one request through the middleware and return the labels recorded on
/// `heraldq_http_requests_total`, as (key, value) pairs.
///
/// The recorder is installed thread-locally rather than globally: `metrics`
/// permits exactly one global recorder per process, and cargo runs tests in
/// parallel threads within a single process, so a global install would have
/// tests clobbering each other. A current-thread runtime keeps the request
/// future on the same thread the recorder is bound to.
fn labels_for_method(method: Method, uri: &str) -> Vec<(String, String)> {
    let app = Router::new()
        .route("/v1/webhook/{id}", get(|| async { "ok" }))
        .layer(middleware::from_fn(track_metrics));

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime");

    metrics::with_local_recorder(&recorder, || {
        rt.block_on(async {
            let req = Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .expect("build request");
            app.oneshot(req).await.expect("route request");
        });
    });

    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .find(|(key, _, _, value)| {
            key.key().name() == "heraldq_http_requests_total"
                && matches!(value, DebugValue::Counter(_))
        })
        .map(|(key, _, _, _)| {
            key.key()
                .labels()
                .map(|l| (l.key().to_owned(), l.value().to_owned()))
                .collect()
        })
        .expect("no heraldq_http_requests_total counter was recorded")
}

fn labels_for(uri: &str) -> Vec<(String, String)> {
    labels_for_method(Method::GET, uri)
}

/// `Method::from_bytes` accepts any valid HTTP token, which is the whole point.
fn junk_method(token: &str) -> Method {
    Method::from_bytes(token.as_bytes()).expect("a valid HTTP token")
}

fn label(labels: &[(String, String)], name: &str) -> String {
    labels
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| panic!("missing label `{name}`"))
}

/// The one that matters: a dynamic segment must collapse to its template.
#[test]
fn path_label_is_the_route_template_not_the_ulid() {
    let ulid = "01KX9K6AD5FRHNYEHNM3E3GNHV";
    let labels = labels_for(&format!("/v1/webhook/{ulid}"));

    let path = label(&labels, "path");
    assert_eq!(
        path, "/v1/webhook/{id}",
        "path label must be the route template, not the concrete URI"
    );
    assert!(
        !path.contains(ulid),
        "the ULID leaked into the `path` label — this is a cardinality explosion, \
         one time series per webhook id"
    );
}

/// Two distinct ids must land on the same series, which is the whole point of
/// the template — the assertion above would still pass if the label were some
/// other constant, so pin the collapse directly.
#[test]
fn distinct_ids_share_one_series() {
    let a = labels_for("/v1/webhook/01KX9K6AD5FRHNYEHNM3E3GNHV");
    let b = labels_for("/v1/webhook/01KX492P4K675BYDDDA6C24343");

    assert_eq!(
        label(&a, "path"),
        label(&b, "path"),
        "different webhook ids must share a single time series"
    );
}

/// Unrouted paths are the other unbounded source: a scanner probing random
/// URLs would otherwise mint a series per probe.
#[test]
fn unmatched_route_collapses_to_unknown() {
    let labels = labels_for("/nope/random-junk-12345");
    assert_eq!(label(&labels, "path"), "unknown");
}

/// Status and method are what make the counter usable as an error-rate signal;
/// the `HeraldQApiErrorRate` alert filters on `status=~"5.."`.
#[test]
fn counter_carries_method_and_status() {
    let labels = labels_for("/v1/webhook/01KX9K6AD5FRHNYEHNM3E3GNHV");
    assert_eq!(label(&labels, "method"), "GET");
    assert_eq!(label(&labels, "status"), "200");
}

/// The `method` label is the *other* unbounded source, and the easy one to miss:
/// `http::Method` is not an enum of the seven verbs — it has an `ExtensionInline`
/// variant that accepts any HTTP token. `PROPFIND`, `MOO`, anything. Left raw,
/// a scanner spraying junk verbs mints one permanent time series per verb.
#[test]
fn junk_method_collapses_to_other() {
    let labels = labels_for_method(junk_method("MOO"), "/v1/webhook/01KX9K6AD5FRHNYEHNM3E3GNHV");

    let method = label(&labels, "method");
    assert_eq!(method, "other", "non-standard methods must collapse");
    assert!(
        !method.contains("MOO"),
        "the raw method token leaked into the `method` label — this is a \
         cardinality explosion, one time series per verb a scanner invents"
    );
}

/// As with the route template: collapsing is only useful if distinct junk verbs
/// land on the *same* series. The assertion above would still pass if each verb
/// mapped to some other unique constant.
#[test]
fn distinct_junk_methods_share_one_series() {
    let a = labels_for_method(junk_method("MOO"), "/v1/webhook/01KX9K6AD5FRHNYEHNM3E3GNHV");
    let b = labels_for_method(
        junk_method("PROPFIND"),
        "/v1/webhook/01KX9K6AD5FRHNYEHNM3E3GNHV",
    );

    assert_eq!(
        label(&a, "method"),
        label(&b, "method"),
        "different junk verbs must share a single time series"
    );
}

/// ...while the standard verbs must still pass through distinctly — collapsing
/// everything to "other" would be "safe" and useless.
#[test]
fn standard_methods_are_preserved() {
    let get = labels_for_method(Method::GET, "/v1/webhook/01KX9K6AD5FRHNYEHNM3E3GNHV");
    let post = labels_for_method(Method::POST, "/v1/webhook/01KX9K6AD5FRHNYEHNM3E3GNHV");

    assert_eq!(label(&get, "method"), "GET");
    assert_eq!(label(&post, "method"), "POST");
}
