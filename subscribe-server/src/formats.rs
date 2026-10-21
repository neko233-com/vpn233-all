//! Public format capability endpoint (`GET /api/v1/formats`).
//!
//! The capability matrix is owned by `vpn233-core-lib-rust` so the server, the
//! admin UI, and the clients all describe formats identically.

use std::collections::BTreeSet;

use axum::{
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use vpn233_core_lib_rust::format_capabilities;

/// Public, unauthenticated capability description used by the admin UI.
pub(crate) async fn formats() -> Response {
    let capabilities = format_capabilities();
    let mut protocols = BTreeSet::new();
    for capability in &capabilities {
        for protocol in &capability.protocols {
            protocols.insert(protocol.clone());
        }
    }
    let formats = capabilities
        .iter()
        .map(|capability| {
            json!({
                "id": capability.id,
                "name": capability.name,
                "content_type": capability.content_type,
                "extension": capability.extension,
                "protocols": capability.protocols,
                "recommended": capability.recommended,
            })
        })
        .collect::<Vec<_>>();
    let mut response = Json(json!({
        "formats": formats,
        "protocols": protocols.into_iter().collect::<Vec<_>>(),
    }))
    .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    response
}
