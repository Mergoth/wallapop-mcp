//! End-to-end tests against the real Wallapop API.
//!
//! All tests are `#[ignore]` and are skipped by default.
//! Run them with:
//!
//!   cargo test --test e2e -- --include-ignored
//!
//! Requirements:
//!   WALLAPOP_SESSION_PATH  — path to a valid session.json with live tokens
//!
//! Each test publishes a real listing and deletes it immediately.
//! If a test panics after creation but before deletion, run:
//!   cargo run -- delete <item_id> --yes

mod common;
use common::*;
use image::{RgbImage, codecs::jpeg::JpegEncoder};
use serde_json::json;
use std::{env, path::PathBuf, time::Duration};
use wallapop_mcp::{
    Context,
    client::{Client, Config},
};

fn session_path() -> Option<PathBuf> {
    env::var_os("WALLAPOP_SESSION_PATH").map(PathBuf::from)
}

fn e2e_client() -> Client {
    let path = session_path().expect(
        "WALLAPOP_SESSION_PATH must be set to run e2e tests",
    );
    Client::new(Config {
        session: Some(path),
        allow_writes: true,
        interval: Some(Duration::from_secs(1)),
        ..Config::default()
    })
    .expect("failed to create e2e client")
}

fn ctx() -> Context {
    Context::new(Duration::from_secs(60))
}

fn small_jpeg_file(dir: &Temp) -> std::path::PathBuf {
    let img = RgbImage::new(4, 4);
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut out, 50)
        .encode(img.as_raw(), 4, 4, image::ExtendedColorType::Rgb8)
        .unwrap();
    dir.file("photo.jpg", out)
}

/// Helper: publish the given args, assert success, return item_id.
/// Panics with the full error if publish fails so the CI log is useful.
fn publish_and_cleanup(c: &Client, args: serde_json::Value) {
    let v = c
        .invoke(&ctx(), "publish_item", &args)
        .unwrap_or_else(|e| panic!("publish_item failed: {e:#?}"));
    let id = v["item_id"].as_str().expect("missing item_id");
    eprintln!("e2e: created {id}, deleting...");
    c.invoke(&ctx(), "delete_item", &json!({"item_id": id, "yes": true}))
        .unwrap_or_else(|e| panic!("delete_item {id} failed: {e:#?}"));
    eprintln!("e2e: deleted {id} ✓");
}

// ── Scenarios ────────────────────────────────────────────────────────────────

/// Category without a color attribute (dolls = 10082).
/// Must reach Wallapop and succeed with no `color` field.
#[test]
#[ignore]
fn e2e_publish_no_color_category() {
    let dir = Temp::new();
    let c = e2e_client();
    publish_and_cleanup(
        &c,
        json!({
            "title": "Test doll [e2e, delete me]",
            "description": "Automated e2e test listing. Safe to delete.",
            "price": "1.00",
            "category_leaf_id": "10082",
            "condition": "good",
            "shipping_allowed": false,
            "photo_paths": [small_jpeg_file(&dir)]
        }),
    );
}

/// Category with a color attribute (cameras = 10088, same as unit tests).
/// Must send color and succeed.
#[test]
#[ignore]
fn e2e_publish_with_color() {
    let dir = Temp::new();
    let c = e2e_client();
    publish_and_cleanup(
        &c,
        json!({
            "title": "Test camera [e2e, delete me]",
            "description": "Automated e2e test listing. Safe to delete.",
            "price": "1.00",
            "category_leaf_id": "10088",
            "condition": "good",
            "color": "black",
            "shipping_allowed": false,
            "photo_paths": [small_jpeg_file(&dir)]
        }),
    );
}

/// Publish with brand + model — both optional, must not be blocked.
#[test]
#[ignore]
fn e2e_publish_with_brand_and_model() {
    let dir = Temp::new();
    let c = e2e_client();
    publish_and_cleanup(
        &c,
        json!({
            "title": "Test camera [e2e, delete me]",
            "description": "Automated e2e test listing. Safe to delete.",
            "price": "1.00",
            "category_leaf_id": "10088",
            "condition": "good",
            "color": "black",
            "brand": "TestBrand",
            "model": "TestModel",
            "shipping_allowed": false,
            "photo_paths": [small_jpeg_file(&dir)]
        }),
    );
}

/// Wallapop API errors must include detail.code (e.g. INVALID_ATTRIBUTE).
/// Uses a known-bad input (empty price string bypasses the schema check
/// by going through the binary protocol) to verify the detail path.
/// This is a schema-level guard test; it does NOT create a listing.
#[test]
#[ignore]
fn e2e_api_error_surfaces_detail() {
    let dir = Temp::new();
    let c = e2e_client();
    // Trigger a real Wallapop 400 by sending an invalid category leaf id
    // (a string that passes our schema but Wallapop won't accept).
    let err = c
        .invoke(
            &ctx(),
            "publish_item",
            &json!({
                "title": "e2e error detail test",
                "description": "This should fail at Wallapop.",
                "price": "1.00",
                "category_leaf_id": "99999999",
                "condition": "good",
                "shipping_allowed": false,
                "photo_paths": [small_jpeg_file(&dir)]
            }),
        )
        .expect_err("expected Wallapop to reject invalid category");
    assert_eq!(err.status, Some(400), "expected HTTP 400 from Wallapop");
    assert!(
        err.detail.is_some(),
        "detail must be populated from Wallapop error body; got: {err:#?}"
    );
    eprintln!(
        "e2e: Wallapop error detail: {}",
        err.detail.as_ref().unwrap()
    );
}
