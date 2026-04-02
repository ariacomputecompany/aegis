use aegis::{
    BatchRequest, BrowserConfig, BrowserMode, Command, CommandMatcher, CommandTarget, Cookie,
    NetworkOverride, RuntimeEvent, SessionState, StorageArea, TraceRecorder,
    dom::diff::{DomMutation, diff_snapshots},
    dom::node::{DomNode, DomNodeSemantics, DomSnapshot},
    events::stream::{EventStream, EventType, SequencedEvent},
    replay_trace,
    transport::protocol::{
        BatchWireResponse, EvalJsRequest, MessageKind, NavigateResponse, TraceFile, decode_message,
        encode_message,
    },
};
use std::collections::HashMap;
use std::env;

#[test]
fn encodes_batch_request_with_stable_shape() {
    let request = BatchRequest {
        batch_id: 42,
        commands: vec![
            Command::Click {
                target: CommandTarget::Id { id: 9 },
            },
            Command::Hover {
                target: CommandTarget::Match {
                    matcher: CommandMatcher {
                        role: Some("button".into()),
                        name: Some("Open".into()),
                        label: None,
                        control_type: None,
                        tag: None,
                        text: None,
                        placeholder: None,
                        href_contains: None,
                        actionable: Some(true),
                        disabled: Some(false),
                        exact: Some(true),
                    },
                },
            },
            Command::SetValue {
                target: CommandTarget::Match {
                    matcher: CommandMatcher {
                        control_type: Some("searchbox".into()),
                        name: Some("Search".into()),
                        role: None,
                        label: None,
                        tag: None,
                        text: None,
                        placeholder: None,
                        href_contains: None,
                        actionable: Some(true),
                        disabled: Some(false),
                        exact: None,
                    },
                },
                value: "hello".into(),
            },
            Command::PressKey {
                target: Some(CommandTarget::Id { id: 9 }),
                key: "Enter".into(),
                code: Some("Enter".into()),
                alt_key: false,
                ctrl_key: false,
                meta_key: false,
                shift_key: false,
            },
            Command::WaitFor {
                target: Some(CommandTarget::Match {
                    matcher: CommandMatcher {
                        role: None,
                        name: Some("Results".into()),
                        label: None,
                        control_type: None,
                        tag: None,
                        text: None,
                        placeholder: None,
                        href_contains: None,
                        actionable: Some(true),
                        disabled: Some(false),
                        exact: None,
                    },
                }),
                url_contains: Some("search".into()),
                title_contains: Some("Results".into()),
                text: Some("browser automation".into()),
                ready_state: Some("complete".into()),
                timeout_ms: Some(1_500),
                poll_interval_ms: Some(25),
            },
            Command::Scroll { x: 0, y: 480 },
        ],
    };

    let encoded = aegis::commands::encoder::encode_batch(&request).expect("batch encodes");
    assert!(encoded.contains("\"batch_id\":42"));
    assert!(encoded.contains("\"type\":\"click\""));
    assert!(encoded.contains("\"type\":\"hover\""));
    assert!(encoded.contains("\"type\":\"set_value\""));
    assert!(encoded.contains("\"type\":\"press_key\""));
    assert!(encoded.contains("\"type\":\"wait_for\""));
    assert!(encoded.contains("\"type\":\"scroll\""));
    assert!(encoded.contains("\"match\""));
    assert!(encoded.contains("\"control_type\":\"searchbox\""));
    assert!(encoded.contains("\"exact\":true"));
}

#[test]
fn diffs_snapshots_for_attribute_changes() {
    let before = DomSnapshot {
        nodes: vec![DomNode {
            id: 3,
            tag: "input".into(),
            attrs: HashMap::from([("type".into(), "text".into())]),
            text: None,
            semantic: None,
            children: vec![],
        }],
    };
    let after = DomSnapshot {
        nodes: vec![DomNode {
            id: 3,
            tag: "input".into(),
            attrs: HashMap::from([
                ("type".into(), "text".into()),
                ("value".into(), "value-1".into()),
            ]),
            text: None,
            semantic: None,
            children: vec![],
        }],
    };
    let changes = diff_snapshots(&before, &after);
    assert_eq!(
        changes,
        vec![DomMutation::SetAttr {
            id: 3,
            name: "value".into(),
            value: Some("value-1".into())
        }]
    );
}

#[test]
fn validates_session_state() {
    let mut session = SessionState::default()
        .with_storage(StorageArea::Local, "token", "abc123")
        .with_storage(StorageArea::Session, "tab", "primary");
    session.network_overrides.push(NetworkOverride {
        header: "Authorization".into(),
        value: "Bearer test".into(),
    });
    session.cookies.push(Cookie {
        name: "sid".into(),
        value: "cookie".into(),
        domain: "example.com".into(),
        path: Some("/".into()),
        expires_unix: None,
        secure: true,
        http_only: true,
    });

    assert!(session.validate().is_ok());
}

#[test]
fn filters_event_stream_by_type() {
    let mut stream = EventStream::default();
    stream.push(SequencedEvent {
        sequence: 1,
        timestamp_ms: 10,
        event: RuntimeEvent::Navigation {
            url: "https://example.com".into(),
        },
    });
    stream.push(SequencedEvent {
        sequence: 2,
        timestamp_ms: 11,
        event: RuntimeEvent::Log {
            level: "info".into(),
            message: "ready".into(),
            data: None,
        },
    });

    let only_nav = stream.read_from(0, Some(EventType::Navigation));
    assert!(!only_nav.gap_detected);
    assert_eq!(only_nav.events.len(), 1);
    assert_eq!(only_nav.events[0].sequence, 1);
}

#[test]
fn network_event_round_trips_extended_shape() {
    let event = RuntimeEvent::Network {
        request_id: "42".into(),
        url: "https://example.com/api".into(),
        method: Some("POST".into()),
        status_code: Some(201),
        status_text: Some("Created".into()),
        mime_type: Some("application/json".into()),
        request_status: Some("success".into()),
        content_length_bytes: Some(128),
        received_content_length_bytes: Some(96),
        duration_ms: Some(42),
        redirect_url: Some("https://example.com/login".into()),
        error_code: None,
        error_text: None,
        is_main_frame: Some(false),
        response_headers: Some(HashMap::from([
            ("content-type".into(), "application/json".into()),
            ("cache-control".into(), "no-cache".into()),
        ]).into_iter().collect()),
    };

    let json = serde_json::to_string(&event).expect("event serializes");
    let decoded: RuntimeEvent = serde_json::from_str(&json).expect("event deserializes");
    assert_eq!(decoded, event);
}

#[test]
fn detects_event_gaps_when_history_is_truncated() {
    let mut stream = EventStream::with_max_retained(2);
    for sequence in 1..=3 {
        stream.push(SequencedEvent {
            sequence,
            timestamp_ms: sequence,
            event: RuntimeEvent::Log {
                level: "info".into(),
                message: format!("event-{sequence}"),
                data: None,
            },
        });
    }

    let window = stream.read_from(0, None);
    assert!(window.gap_detected);
    assert_eq!(window.oldest_available_sequence, Some(2));
    assert_eq!(window.latest_sequence, 3);
    assert_eq!(window.events.len(), 2);
}

#[test]
fn binary_protocol_round_trips() {
    let frame = encode_message(
        MessageKind::EvalJs,
        &EvalJsRequest {
            script: "return document.title".into(),
        },
    )
    .expect("frame encodes");

    let payload: EvalJsRequest =
        decode_message(MessageKind::EvalJs, &frame).expect("frame decodes");
    assert_eq!(payload.script, "return document.title");
}

#[test]
fn batch_wire_response_decodes_null_snapshot() {
    let frame = encode_message(
        MessageKind::SendBatch,
        &BatchWireResponse {
            batch_id: 5,
            results: Vec::new(),
            snapshot: None,
            events: Vec::new(),
        },
    )
    .expect("frame encodes");

    let payload: BatchWireResponse =
        decode_message(MessageKind::SendBatch, &frame).expect("frame decodes");
    assert_eq!(payload.batch_id, 5);
    assert!(payload.snapshot.is_none());
}

#[test]
fn navigate_response_decodes_null_snapshot() {
    let frame = encode_message(
        MessageKind::Navigate,
        &NavigateResponse {
            url: "https://example.com".into(),
            snapshot: None,
            events: Vec::new(),
        },
    )
    .expect("frame encodes");

    let payload: NavigateResponse =
        decode_message(MessageKind::Navigate, &frame).expect("frame decodes");
    assert_eq!(payload.url, "https://example.com");
    assert!(payload.snapshot.is_none());
}

#[test]
fn trace_recorder_persists_batches() {
    let path = env::temp_dir().join("aegis-trace-test.json");
    let _ = std::fs::remove_file(&path);

    let mut recorder = TraceRecorder::new(
        &path,
        BrowserConfig {
            mode: BrowserMode::Headless,
            start_url: None,
        },
    );
    recorder.set_initial_session(SessionState::default());
    recorder.record_batch(
        BatchRequest {
            batch_id: 7,
            commands: vec![Command::Click {
                target: CommandTarget::Id { id: 3 },
            }],
        },
        aegis::BatchResponse {
            batch_id: 7,
            results: vec![aegis::CommandResult::ok(serde_json::json!({"clicked": 3}))],
            snapshot: Some(DomSnapshot::default()),
            events: Vec::new(),
        },
        &[SequencedEvent {
            sequence: 1,
            timestamp_ms: 1,
            event: RuntimeEvent::Log {
                level: "info".into(),
                message: "batch".into(),
                data: None,
            },
        }],
    );
    recorder.flush().expect("trace flushes");

    let loaded = TraceRecorder::load(&path).expect("trace loads");
    let trace: &TraceFile = loaded.trace();
    assert_eq!(trace.batches.len(), 1);
    assert_eq!(trace.batches[0].batch_id, 7);
    assert_eq!(trace.browser_config.mode, BrowserMode::Headless);
}

#[test]
fn replay_trace_rebuilds_final_state() {
    let path = env::temp_dir().join("aegis-trace-replay-test.json");
    let _ = std::fs::remove_file(&path);

    let mut recorder = TraceRecorder::new(
        &path,
        BrowserConfig {
            mode: BrowserMode::Headful,
            start_url: Some("https://example.com".into()),
        },
    );
    recorder.record_batch(
        BatchRequest {
            batch_id: 9,
            commands: vec![],
        },
        aegis::BatchResponse {
            batch_id: 9,
            results: Vec::new(),
            snapshot: Some(DomSnapshot {
                nodes: vec![DomNode {
                    id: 1,
                    tag: "html".into(),
                    attrs: HashMap::new(),
                    text: None,
                    semantic: None,
                    children: vec![],
                }],
            }),
            events: Vec::new(),
        },
        &[SequencedEvent {
            sequence: 2,
            timestamp_ms: 2,
            event: RuntimeEvent::Navigation {
                url: "https://example.com".into(),
            },
        }],
    );
    recorder.flush().expect("trace flushes");

    let replay = replay_trace(&path).expect("trace replays");
    assert_eq!(replay.final_snapshot.nodes.len(), 1);
    assert_eq!(replay.events.latest_sequence(), 2);
    assert_eq!(replay.browser_config.mode, BrowserMode::Headful);
}

#[test]
fn diffs_snapshots_for_semantic_changes() {
    let before = DomSnapshot {
        nodes: vec![DomNode {
            id: 7,
            tag: "button".into(),
            attrs: HashMap::new(),
            text: Some("Search".into()),
            semantic: Some(DomNodeSemantics {
                role: Some("button".into()),
                name: Some("Search".into()),
                label: None,
                control_type: Some("button".into()),
                actionable: true,
                disabled: false,
                visible: true,
                actions: vec!["click".into()],
            }),
            children: vec![],
        }],
    };
    let after = DomSnapshot {
        nodes: vec![DomNode {
            id: 7,
            tag: "button".into(),
            attrs: HashMap::new(),
            text: Some("Search".into()),
            semantic: Some(DomNodeSemantics {
                role: Some("button".into()),
                name: Some("Submit search".into()),
                label: None,
                control_type: Some("button".into()),
                actionable: true,
                disabled: false,
                visible: true,
                actions: vec!["click".into(), "submit".into()],
            }),
            children: vec![],
        }],
    };

    let changes = diff_snapshots(&before, &after);
    assert_eq!(
        changes,
        vec![DomMutation::Upsert {
            id: 7,
            tag: "button".into(),
            attrs: HashMap::new(),
            text: Some("Search".into()),
            semantic: Some(DomNodeSemantics {
                role: Some("button".into()),
                name: Some("Submit search".into()),
                label: None,
                control_type: Some("button".into()),
                actionable: true,
                disabled: false,
                visible: true,
                actions: vec!["click".into(), "submit".into()],
            }),
            children: vec![],
        }]
    );
}

#[test]
fn replay_trace_retains_last_non_null_snapshot() {
    let path = env::temp_dir().join("aegis-trace-replay-null-snapshot-test.json");
    let _ = std::fs::remove_file(&path);

    let mut recorder = TraceRecorder::new(
        &path,
        BrowserConfig {
            mode: BrowserMode::Headful,
            start_url: Some("https://example.com".into()),
        },
    );
    recorder.record_batch(
        BatchRequest {
            batch_id: 1,
            commands: vec![],
        },
        aegis::BatchResponse {
            batch_id: 1,
            results: Vec::new(),
            snapshot: Some(DomSnapshot {
                nodes: vec![DomNode {
                    id: 1,
                    tag: "html".into(),
                    attrs: HashMap::new(),
                    text: None,
                    semantic: None,
                    children: vec![2],
                }],
            }),
            events: Vec::new(),
        },
        &[],
    );
    recorder.record_batch(
        BatchRequest {
            batch_id: 2,
            commands: vec![Command::Scroll { x: 0, y: 480 }],
        },
        aegis::BatchResponse {
            batch_id: 2,
            results: Vec::new(),
            snapshot: None,
            events: Vec::new(),
        },
        &[],
    );
    recorder.flush().expect("trace flushes");

    let replay = replay_trace(&path).expect("trace replays");
    assert_eq!(replay.final_snapshot.nodes.len(), 1);
    assert_eq!(replay.final_snapshot.nodes[0].tag, "html");
    assert_eq!(replay.final_snapshot.nodes[0].children, vec![2]);
}

#[test]
fn replay_trace_clears_snapshot_on_navigation_without_snapshot() {
    let path = env::temp_dir().join("aegis-trace-replay-nav-clear-test.json");
    let _ = std::fs::remove_file(&path);

    let mut recorder = TraceRecorder::new(
        &path,
        BrowserConfig {
            mode: BrowserMode::Headful,
            start_url: Some("https://example.com".into()),
        },
    );
    recorder.record_batch(
        BatchRequest {
            batch_id: 1,
            commands: vec![],
        },
        aegis::BatchResponse {
            batch_id: 1,
            results: Vec::new(),
            snapshot: Some(DomSnapshot {
                nodes: vec![DomNode {
                    id: 1,
                    tag: "html".into(),
                    attrs: HashMap::new(),
                    text: None,
                    semantic: None,
                    children: vec![2],
                }],
            }),
            events: Vec::new(),
        },
        &[],
    );
    recorder.record_batch(
        BatchRequest {
            batch_id: 2,
            commands: vec![],
        },
        aegis::BatchResponse {
            batch_id: 2,
            results: Vec::new(),
            snapshot: None,
            events: Vec::new(),
        },
        &[SequencedEvent {
            sequence: 3,
            timestamp_ms: 3,
            event: RuntimeEvent::Navigation {
                url: "https://www.wikipedia.org".into(),
            },
        }],
    );
    recorder.flush().expect("trace flushes");

    let replay = replay_trace(&path).expect("trace replays");
    assert!(replay.final_snapshot.nodes.is_empty());
    assert_eq!(replay.events.latest_sequence(), 3);
}

#[test]
fn browser_config_serializes_mode() {
    let json = serde_json::to_string(&BrowserConfig {
        mode: BrowserMode::Headful,
        start_url: None,
    })
    .expect("config serializes");

    assert!(json.contains("\"mode\":\"headful\""));
}
