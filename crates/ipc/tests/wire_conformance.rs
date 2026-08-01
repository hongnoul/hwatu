// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong

use hwatu_ipc::{Event, LoadStage, OpenMode, Request, Response};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

fn assert_golden_roundtrip<T>(fixture: &str)
where
    T: DeserializeOwned + Serialize,
{
    let expected: Value = serde_json::from_str(fixture).expect("fixture must be valid JSON");
    let message: T = serde_json::from_value(expected.clone()).expect("fixture must match protocol");
    let actual = serde_json::to_value(message).expect("protocol value must serialize");
    assert_eq!(actual, expected);
}

#[test]
fn legacy_requests_keep_their_defaults() {
    let Request::Open { url, app_id, mode } =
        serde_json::from_str(include_str!("fixtures/requests/legacy_open.json"))
            .expect("legacy open request must parse")
    else {
        panic!("legacy open fixture parsed as the wrong command");
    };
    assert_eq!(url.as_deref(), Some("https://example.test/"));
    assert_eq!(app_id, None);
    assert_eq!(mode, OpenMode::Normal);

    let Request::Navigate {
        id,
        url,
        wait,
        until,
        timeout_ms,
    } = serde_json::from_str(include_str!("fixtures/requests/legacy_navigate.json"))
        .expect("legacy navigate request must parse")
    else {
        panic!("legacy navigate fixture parsed as the wrong command");
    };
    assert_eq!(id, None);
    assert_eq!(url, "https://example.test/dashboard");
    assert!(wait);
    assert_eq!(until, LoadStage::Settled);
    assert_eq!(timeout_ms, None);

    let Request::Check {
        url,
        render,
        base,
        shot,
        until,
        keep,
        ..
    } = serde_json::from_str(include_str!("fixtures/requests/legacy_check.json"))
        .expect("legacy check request must parse")
    else {
        panic!("legacy check fixture parsed as the wrong command");
    };
    assert_eq!(url.as_deref(), Some("http://localhost:3000"));
    assert_eq!(render, None);
    assert_eq!(base, None);
    assert!(shot);
    assert_eq!(until, LoadStage::Settled);
    assert!(!keep);
}

#[test]
fn minimal_subscription_remains_valid() {
    let Request::Subscribe { kinds, window } =
        serde_json::from_str(include_str!("fixtures/requests/minimal_subscribe.json"))
            .expect("minimal subscribe request must parse")
    else {
        panic!("subscribe fixture parsed as the wrong command");
    };
    assert_eq!(kinds, None);
    assert_eq!(window, None);
}

#[test]
fn persistent_connections_are_newline_delimited_request_sequences() {
    let wire = "{\"cmd\":\"ping\"}\n{\"cmd\":\"list\"}\n";
    let requests: Vec<Request> = wire
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is one request"))
        .collect();

    assert!(matches!(
        requests.as_slice(),
        [Request::Ping, Request::List]
    ));
}

#[test]
fn canonical_responses_match_the_wire() {
    assert_golden_roundtrip::<Response>(include_str!("fixtures/responses/window.json"));
    assert_golden_roundtrip::<Response>(include_str!("fixtures/responses/value.json"));
}

#[test]
fn structured_errors_match_the_wire() {
    let fixture = include_str!("fixtures/responses/error.json");
    assert_golden_roundtrip::<Response>(fixture);

    let Response::Err { message } =
        serde_json::from_str(fixture).expect("structured error response must parse")
    else {
        panic!("error fixture parsed as a successful response");
    };
    assert_eq!(message, "window 42 not found");
}

#[test]
fn canonical_events_match_the_wire() {
    assert_golden_roundtrip::<Event>(include_str!("fixtures/events/load.json"));
    assert_golden_roundtrip::<Event>(include_str!("fixtures/events/subscribed.json"));
}
