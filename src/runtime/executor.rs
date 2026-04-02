use crate::browser::BrowserConfig;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::command::{Command, CommandResult, CommandTarget};
use crate::commands::matcher::{DesiredAction, resolve_command_target};
use crate::dom::diff::DomMutation;
use crate::dom::node::DomSnapshot;
use crate::dom::tree::DomTree;
use crate::events::stream::{EventReadWindow, EventStream, RuntimeEvent, SequencedEvent};
use crate::runtime::scheduler::Scheduler;
use crate::session::cookies::SessionState;
use crate::trace::recorder::TraceRecorder;
use crate::transport::bridge::{
    AegisError, BatchRequest, BatchResponse, BridgeEventEnvelope, CefBridge,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatus {
    pub bootstrapped: bool,
    pub bootstrap_duration_ms: Option<u64>,
    pub dom_nodes: usize,
    pub dom_snapshot_available: bool,
    pub retained_event_count: usize,
    pub latest_event_sequence: u64,
    pub oldest_retained_event_sequence: Option<u64>,
    pub current_url: Option<String>,
    pub current_title: Option<String>,
    pub document_ready_state: Option<String>,
    pub last_dom_refresh_at_ms: Option<u64>,
    pub last_live_state_refresh_at_ms: Option<u64>,
    pub last_event_at_ms: Option<u64>,
    pub last_successful_command_at_ms: Option<u64>,
    pub last_successful_bridge_roundtrip_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReport {
    pub batch_id: u64,
    pub results: Vec<CommandResult>,
    pub latest_event_sequence: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DomTelemetrySummary {
    pub total_nodes: usize,
    pub actionable_nodes: usize,
    pub visible_nodes: usize,
    pub disabled_nodes: usize,
    pub text_nodes: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecentNavigationTelemetry {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecentNetworkRequestTelemetry {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub request_id: String,
    pub url: String,
    pub method: Option<String>,
    pub status_code: Option<u16>,
    pub status_text: Option<String>,
    pub mime_type: Option<String>,
    pub request_status: Option<String>,
    pub content_length_bytes: Option<i64>,
    pub received_content_length_bytes: Option<i64>,
    pub duration_ms: Option<u64>,
    pub redirect_url: Option<String>,
    pub error_code: Option<i32>,
    pub error_text: Option<String>,
    pub is_main_frame: Option<bool>,
    pub response_headers: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecentLogTelemetry {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub level: String,
    pub message: String,
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventTelemetrySummary {
    pub total_events: u64,
    pub dom_mutation_events: u64,
    pub dom_mutation_changes: u64,
    pub navigation_events: u64,
    pub network_events: u64,
    pub log_events: u64,
    pub network_summary: NetworkTelemetrySummary,
    pub recent_navigations: Vec<RecentNavigationTelemetry>,
    pub recent_network_requests: Vec<RecentNetworkRequestTelemetry>,
    pub recent_logs: Vec<RecentLogTelemetry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkDomainTelemetry {
    pub host: String,
    pub request_count: u64,
    pub failure_count: u64,
    pub redirect_count: u64,
    pub transferred_bytes: u64,
    pub avg_duration_ms: Option<u64>,
    pub max_duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkBreakdownTelemetry {
    pub key: String,
    pub count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkTelemetrySummary {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub redirected_requests: u64,
    pub main_frame_requests: u64,
    pub informational_responses: u64,
    pub successful_responses: u64,
    pub redirect_responses: u64,
    pub client_error_responses: u64,
    pub server_error_responses: u64,
    pub transferred_bytes: u64,
    pub avg_duration_ms: Option<u64>,
    pub max_duration_ms: Option<u64>,
    pub method_breakdown: Vec<NetworkBreakdownTelemetry>,
    pub mime_breakdown: Vec<NetworkBreakdownTelemetry>,
    pub status_code_breakdown: Vec<NetworkBreakdownTelemetry>,
    pub top_errors: Vec<NetworkBreakdownTelemetry>,
    pub top_domains: Vec<NetworkDomainTelemetry>,
}

#[derive(Debug, Clone, Default)]
struct NetworkDomainAggregate {
    request_count: u64,
    failure_count: u64,
    redirect_count: u64,
    transferred_bytes: u64,
    total_duration_ms: u64,
    duration_count: u64,
    max_duration_ms: u64,
}

#[derive(Debug, Clone, Default)]
struct NetworkBreakdownAggregate {
    count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageViewportTelemetry {
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub device_pixel_ratio: Option<f64>,
    pub scroll_x: Option<f64>,
    pub scroll_y: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageNavigationTelemetry {
    pub navigation_type: Option<String>,
    pub dom_content_loaded_ms: Option<f64>,
    pub load_event_ms: Option<f64>,
    pub dom_interactive_ms: Option<f64>,
    pub response_end_ms: Option<f64>,
    pub transfer_size_bytes: Option<u64>,
    pub encoded_body_size_bytes: Option<u64>,
    pub decoded_body_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageResourceTelemetry {
    pub resource_count: u64,
    pub script_count: u64,
    pub stylesheet_count: u64,
    pub image_count: u64,
    pub fetch_count: u64,
    pub xml_http_request_count: u64,
    pub other_count: u64,
    pub transfer_size_bytes: Option<u64>,
    pub encoded_body_size_bytes: Option<u64>,
    pub decoded_body_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageJsHeapTelemetry {
    pub used_js_heap_size_bytes: Option<u64>,
    pub total_js_heap_size_bytes: Option<u64>,
    pub js_heap_size_limit_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PagePaintTelemetry {
    pub first_paint_ms: Option<f64>,
    pub first_contentful_paint_ms: Option<f64>,
    pub largest_contentful_paint_ms: Option<f64>,
    pub largest_contentful_paint_size: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageStabilityTelemetry {
    pub cumulative_layout_shift: Option<f64>,
    pub layout_shift_count: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageResponsivenessTelemetry {
    pub long_task_count: u64,
    pub long_task_total_duration_ms: Option<f64>,
    pub long_task_max_duration_ms: Option<f64>,
    pub event_count: u64,
    pub interaction_count: u64,
    pub total_event_duration_ms: Option<f64>,
    pub max_event_duration_ms: Option<f64>,
    pub first_input_delay_ms: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageRuntimeTelemetry {
    pub sampled_at_ms: u64,
    pub url: Option<String>,
    pub title: Option<String>,
    pub ready_state: Option<String>,
    pub origin: Option<String>,
    pub visibility_state: Option<String>,
    pub has_focus: Option<bool>,
    pub viewport: PageViewportTelemetry,
    pub navigation: PageNavigationTelemetry,
    pub resources: PageResourceTelemetry,
    pub js_heap: PageJsHeapTelemetry,
    pub paint: PagePaintTelemetry,
    pub stability: PageStabilityTelemetry,
    pub responsiveness: PageResponsivenessTelemetry,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TraceTelemetry {
    pub enabled: bool,
    pub path: Option<String>,
    pub recorded_batches: usize,
    pub initial_session_captured: bool,
    pub file_size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTelemetrySnapshot {
    pub status: RuntimeStatus,
    pub dom: DomTelemetrySummary,
    pub events: EventTelemetrySummary,
    pub page: Option<PageRuntimeTelemetry>,
    pub page_capture_error: Option<String>,
    pub trace: TraceTelemetry,
}

pub struct AegisRuntime {
    bridge: CefBridge,
    browser_config: BrowserConfig,
    dom: DomTree,
    events: EventStream,
    scheduler: Scheduler,
    trace_recorder: Option<TraceRecorder>,
    runtime_bootstrapped: bool,
    bootstrap_duration_ms: Option<u64>,
    dom_snapshot_valid: bool,
    current_url: Option<String>,
    current_title: Option<String>,
    document_ready_state: Option<String>,
    last_dom_refresh_at_ms: Option<u64>,
    last_live_state_refresh_at_ms: Option<u64>,
    last_event_at_ms: Option<u64>,
    last_successful_command_at_ms: Option<u64>,
    last_successful_bridge_roundtrip_at_ms: Option<u64>,
    total_events: u64,
    dom_mutation_events: u64,
    dom_mutation_changes: u64,
    navigation_events: u64,
    network_events: u64,
    log_events: u64,
    successful_network_requests: u64,
    failed_network_requests: u64,
    redirected_network_requests: u64,
    main_frame_network_requests: u64,
    informational_responses: u64,
    successful_responses: u64,
    redirect_responses: u64,
    client_error_responses: u64,
    server_error_responses: u64,
    transferred_network_bytes: u64,
    total_network_duration_ms: u64,
    network_duration_samples: u64,
    max_network_duration_ms: u64,
    network_domains: BTreeMap<String, NetworkDomainAggregate>,
    network_methods: BTreeMap<String, NetworkBreakdownAggregate>,
    network_mime_types: BTreeMap<String, NetworkBreakdownAggregate>,
    network_status_codes: BTreeMap<String, NetworkBreakdownAggregate>,
    network_errors: BTreeMap<String, NetworkBreakdownAggregate>,
    recent_navigations: VecDeque<RecentNavigationTelemetry>,
    recent_network_requests: VecDeque<RecentNetworkRequestTelemetry>,
    recent_logs: VecDeque<RecentLogTelemetry>,
}

const LIVE_STATE_REFRESH_INTERVAL_MS: u64 = 250;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_WAIT_POLL_INTERVAL_MS: u64 = 50;
const MIN_WAIT_POLL_INTERVAL_MS: u64 = 10;
const RECENT_TELEMETRY_LIMIT: usize = 25;

type PendingBatchFlush = (Vec<CommandResult>, Vec<SequencedEvent>, Option<DomSnapshot>);

impl AegisRuntime {
    pub fn new(
        bridge: CefBridge,
        browser_config: BrowserConfig,
        bootstrap_duration_ms: Option<u64>,
    ) -> Result<Self, AegisError> {
        Ok(Self {
            bridge,
            browser_config,
            dom: DomTree::default(),
            events: EventStream::default(),
            scheduler: Scheduler::default(),
            trace_recorder: None,
            runtime_bootstrapped: bootstrap_duration_ms.is_some(),
            bootstrap_duration_ms,
            dom_snapshot_valid: false,
            current_url: None,
            current_title: None,
            document_ready_state: None,
            last_dom_refresh_at_ms: None,
            last_live_state_refresh_at_ms: None,
            last_event_at_ms: None,
            last_successful_command_at_ms: None,
            last_successful_bridge_roundtrip_at_ms: None,
            total_events: 0,
            dom_mutation_events: 0,
            dom_mutation_changes: 0,
            navigation_events: 0,
            network_events: 0,
            log_events: 0,
            successful_network_requests: 0,
            failed_network_requests: 0,
            redirected_network_requests: 0,
            main_frame_network_requests: 0,
            informational_responses: 0,
            successful_responses: 0,
            redirect_responses: 0,
            client_error_responses: 0,
            server_error_responses: 0,
            transferred_network_bytes: 0,
            total_network_duration_ms: 0,
            network_duration_samples: 0,
            max_network_duration_ms: 0,
            network_domains: BTreeMap::new(),
            network_methods: BTreeMap::new(),
            network_mime_types: BTreeMap::new(),
            network_status_codes: BTreeMap::new(),
            network_errors: BTreeMap::new(),
            recent_navigations: VecDeque::with_capacity(RECENT_TELEMETRY_LIMIT),
            recent_network_requests: VecDeque::with_capacity(RECENT_TELEMETRY_LIMIT),
            recent_logs: VecDeque::with_capacity(RECENT_TELEMETRY_LIMIT),
        })
    }

    pub fn execute(&mut self, commands: &[Command]) -> Result<ExecutionReport, AegisError> {
        self.ensure_runtime_bootstrapped(self.commands_require_dom_snapshot(commands))?;
        let batch_id = self.scheduler.next_batch_id();
        let request = BatchRequest {
            batch_id,
            commands: commands.to_vec(),
        };
        let (response, results, emitted_events) =
            self.execute_command_stream(batch_id, commands)?;
        self.mark_successful_command();
        self.record_trace(request, response, &emitted_events)?;

        Ok(ExecutionReport {
            batch_id,
            results,
            latest_event_sequence: self.events.latest_sequence(),
        })
    }

    pub fn navigate(&mut self, url: String) -> Result<Vec<SequencedEvent>, AegisError> {
        self.ensure_runtime_bootstrapped(false)?;
        let response = self.bridge.navigate(&url)?;
        let request = BatchRequest {
            batch_id: self.scheduler.next_batch_id(),
            commands: Vec::new(),
        };
        let emitted_events = self.apply_response(response.clone())?;
        let _ = self.refresh_live_state(true);
        self.mark_successful_command();
        self.record_trace(request, response, &emitted_events)?;
        Ok(emitted_events)
    }

    fn apply_response(
        &mut self,
        response: BatchResponse,
    ) -> Result<Vec<SequencedEvent>, AegisError> {
        let has_navigation = response
            .events
            .iter()
            .any(|event| matches!(event.event, RuntimeEvent::Navigation { .. }));
        if let Some(snapshot) = response.snapshot.clone() {
            self.dom.replace_snapshot(snapshot);
            self.dom_snapshot_valid = true;
            self.last_dom_refresh_at_ms = Some(now_ms());
        } else if has_navigation {
            self.dom.replace_snapshot(DomSnapshot::default());
            self.dom_snapshot_valid = false;
        }
        if let Some(url) = response
            .events
            .iter()
            .rev()
            .find_map(|event| match &event.event {
                RuntimeEvent::Navigation { url } => Some(url.clone()),
                _ => None,
            })
        {
            self.current_url = Some(url);
        }

        Ok(self.apply_event_batch(response.events))
    }

    fn apply_event_batch(&mut self, raw_events: Vec<BridgeEventEnvelope>) -> Vec<SequencedEvent> {
        self.apply_dom_mutations(&raw_events);

        let events = raw_events
            .into_iter()
            .map(|event| self.sequence_event(event))
            .collect::<Vec<_>>();
        if !events.is_empty() {
            self.last_event_at_ms = Some(now_ms());
        }
        for event in &events {
            self.record_event_telemetry(event);
        }
        self.events.push_all(events.clone());
        events
    }

    fn apply_dom_mutations(&mut self, events: &[BridgeEventEnvelope]) {
        if !self.dom_snapshot_valid {
            return;
        }
        let mut changes = Vec::<DomMutation>::new();
        for event in events {
            if let RuntimeEvent::DomMutation {
                changes: event_changes,
            } = &event.event
            {
                changes.extend(event_changes.iter().cloned());
            }
        }
        if !changes.is_empty() {
            self.dom.apply_mutations(&changes);
        }
    }

    fn sequence_event(&mut self, event: BridgeEventEnvelope) -> SequencedEvent {
        SequencedEvent {
            sequence: self.scheduler.next_event_sequence(),
            timestamp_ms: self.scheduler.next_timestamp_ms(),
            event: event.event,
        }
    }

    fn record_event_telemetry(&mut self, event: &SequencedEvent) {
        self.total_events += 1;
        match &event.event {
            RuntimeEvent::DomMutation { changes } => {
                self.dom_mutation_events += 1;
                self.dom_mutation_changes += changes.len() as u64;
            }
            RuntimeEvent::Navigation { url } => {
                self.navigation_events += 1;
                push_recent(
                    &mut self.recent_navigations,
                    RecentNavigationTelemetry {
                        sequence: event.sequence,
                        timestamp_ms: event.timestamp_ms,
                        url: url.clone(),
                    },
                );
            }
            RuntimeEvent::Network {
                request_id,
                url,
                method,
                status_code,
                status_text,
                mime_type,
                request_status,
                content_length_bytes,
                received_content_length_bytes,
                duration_ms,
                redirect_url,
                error_code,
                error_text,
                is_main_frame,
                response_headers,
            } => {
                self.network_events += 1;
                if matches!(request_status.as_deref(), Some("success")) {
                    self.successful_network_requests += 1;
                }
                if matches!(request_status.as_deref(), Some("failed" | "canceled"))
                    || error_code.is_some()
                {
                    self.failed_network_requests += 1;
                }
                if redirect_url.is_some() {
                    self.redirected_network_requests += 1;
                }
                if is_main_frame == &Some(true) {
                    self.main_frame_network_requests += 1;
                }
                if let Some(status_code) = status_code {
                    match *status_code {
                        100..=199 => self.informational_responses += 1,
                        200..=299 => self.successful_responses += 1,
                        300..=399 => self.redirect_responses += 1,
                        400..=499 => self.client_error_responses += 1,
                        500..=599 => self.server_error_responses += 1,
                        _ => {}
                    }
                }
                if let Some(bytes) =
                    received_content_length_bytes.and_then(|value| u64::try_from(value).ok())
                {
                    self.transferred_network_bytes += bytes;
                }
                if let Some(method) = method.as_ref().map(|value| value.trim()).filter(|value| !value.is_empty()) {
                    self.network_methods
                        .entry(method.to_ascii_uppercase())
                        .or_default()
                        .count += 1;
                }
                if let Some(mime_type) = mime_type.as_ref().map(|value| value.trim()).filter(|value| !value.is_empty()) {
                    self.network_mime_types
                        .entry(normalize_mime_group(mime_type))
                        .or_default()
                        .count += 1;
                }
                if let Some(status_code) = status_code {
                    self.network_status_codes
                        .entry(status_code.to_string())
                        .or_default()
                        .count += 1;
                }
                if let Some(error_key) = build_network_error_key(
                    request_status.as_deref(),
                    error_code.as_ref(),
                    error_text.as_deref(),
                ) {
                    self.network_errors.entry(error_key).or_default().count += 1;
                }
                if let Some(duration_ms) = duration_ms {
                    self.total_network_duration_ms += *duration_ms;
                    self.network_duration_samples += 1;
                    self.max_network_duration_ms = self.max_network_duration_ms.max(*duration_ms);
                }
                if let Some(host) = extract_host(url) {
                    let aggregate = self.network_domains.entry(host).or_default();
                    aggregate.request_count += 1;
                    if matches!(request_status.as_deref(), Some("failed" | "canceled"))
                        || error_code.is_some()
                    {
                        aggregate.failure_count += 1;
                    }
                    if redirect_url.is_some() {
                        aggregate.redirect_count += 1;
                    }
                    if let Some(bytes) =
                        received_content_length_bytes.and_then(|value| u64::try_from(value).ok())
                    {
                        aggregate.transferred_bytes += bytes;
                    }
                    if let Some(duration_ms) = duration_ms {
                        aggregate.total_duration_ms += *duration_ms;
                        aggregate.duration_count += 1;
                        aggregate.max_duration_ms = aggregate.max_duration_ms.max(*duration_ms);
                    }
                }
                push_recent(
                    &mut self.recent_network_requests,
                    RecentNetworkRequestTelemetry {
                        sequence: event.sequence,
                        timestamp_ms: event.timestamp_ms,
                        request_id: request_id.clone(),
                        url: url.clone(),
                        method: method.clone(),
                        status_code: *status_code,
                        status_text: status_text.clone(),
                        mime_type: mime_type.clone(),
                        request_status: request_status.clone(),
                        content_length_bytes: *content_length_bytes,
                        received_content_length_bytes: *received_content_length_bytes,
                        duration_ms: *duration_ms,
                        redirect_url: redirect_url.clone(),
                        error_code: *error_code,
                        error_text: error_text.clone(),
                        is_main_frame: *is_main_frame,
                        response_headers: response_headers.clone(),
                    },
                );
            }
            RuntimeEvent::Log {
                level,
                message,
                data,
            } => {
                self.log_events += 1;
                push_recent(
                    &mut self.recent_logs,
                    RecentLogTelemetry {
                        sequence: event.sequence,
                        timestamp_ms: event.timestamp_ms,
                        level: level.clone(),
                        message: message.clone(),
                        data: data.clone(),
                    },
                );
            }
        }
    }

    pub fn inject_session(&mut self, session: SessionState) -> Result<(), AegisError> {
        session.validate().map_err(AegisError::InvalidSession)?;
        if let Some(recorder) = &mut self.trace_recorder {
            recorder.set_initial_session(session.clone());
            recorder.flush()?;
        }
        self.bridge.inject_session(session)?;
        self.mark_successful_bridge_roundtrip();
        let _ = self.refresh_live_state(true);
        Ok(())
    }

    pub fn snapshot_session(&mut self) -> Result<SessionState, AegisError> {
        self.ensure_runtime_bootstrapped(false)?;
        let session = self.bridge.snapshot_session()?;
        self.mark_successful_bridge_roundtrip();
        let _ = self.refresh_live_state(false);
        Ok(session)
    }

    pub fn pump(&mut self) -> Result<(), AegisError> {
        self.bridge.pump()?;
        let _ = self.drain_pending_events()?;
        let _ = self.refresh_live_state(false);
        Ok(())
    }

    pub fn establish_command_bridge(&mut self) -> Result<(), AegisError> {
        self.bridge.install_runtime()?;
        let raw_events = self.bridge.drain_events()?;
        self.mark_successful_bridge_roundtrip();
        let _ = self.apply_event_batch(raw_events);
        let _ = self.ensure_page_telemetry_probe();
        let _ = self.refresh_live_state(true);
        Ok(())
    }

    pub fn snapshot_dom(&mut self) -> Result<crate::dom::node::DomSnapshot, AegisError> {
        self.refresh_dom_snapshot()?;
        Ok(self.dom.snapshot())
    }

    pub fn event_stream(&self) -> &EventStream {
        &self.events
    }

    pub fn drain_pending_events(&mut self) -> Result<Vec<SequencedEvent>, AegisError> {
        let raw_events = self.bridge.drain_events()?;
        self.mark_successful_bridge_roundtrip();
        Ok(self.apply_event_batch(raw_events))
    }

    pub fn read_events_from(&self, sequence: u64) -> EventReadWindow {
        self.events.read_from(sequence, None)
    }

    pub fn bridge(&self) -> &CefBridge {
        &self.bridge
    }

    pub fn bridge_mut(&mut self) -> &mut CefBridge {
        &mut self.bridge
    }

    pub fn enable_trace_recording(&mut self, path: impl Into<std::path::PathBuf>) {
        self.trace_recorder = Some(TraceRecorder::new(path, self.browser_config.clone()));
    }

    pub fn browser_config(&self) -> &BrowserConfig {
        &self.browser_config
    }

    pub fn snapshot_telemetry(&mut self) -> RuntimeTelemetrySnapshot {
        let _ = self.drain_pending_events();
        let _ = self.refresh_live_state(false);
        let (page, page_capture_error) = match self.capture_page_runtime_telemetry() {
            Ok(page) => (Some(page), None),
            Err(error) => (None, Some(error.to_string())),
        };
        RuntimeTelemetrySnapshot {
            status: self.runtime_status(),
            dom: self.build_dom_telemetry(),
            events: EventTelemetrySummary {
                total_events: self.total_events,
                dom_mutation_events: self.dom_mutation_events,
                dom_mutation_changes: self.dom_mutation_changes,
                navigation_events: self.navigation_events,
                network_events: self.network_events,
                log_events: self.log_events,
                network_summary: self.network_summary(),
                recent_navigations: self.recent_navigations.iter().cloned().collect(),
                recent_network_requests: self.recent_network_requests.iter().cloned().collect(),
                recent_logs: self.recent_logs.iter().cloned().collect(),
            },
            page,
            page_capture_error,
            trace: self.trace_telemetry(),
        }
    }

    pub fn runtime_status(&self) -> RuntimeStatus {
        RuntimeStatus {
            bootstrapped: self.runtime_bootstrapped,
            bootstrap_duration_ms: self.bootstrap_duration_ms,
            dom_nodes: self.dom.snapshot().nodes.len(),
            dom_snapshot_available: self.dom_snapshot_valid,
            retained_event_count: self.events.retained_len(),
            latest_event_sequence: self.events.latest_sequence(),
            oldest_retained_event_sequence: self.events.oldest_sequence(),
            current_url: self.current_url.clone(),
            current_title: self.current_title.clone(),
            document_ready_state: self.document_ready_state.clone(),
            last_dom_refresh_at_ms: self.last_dom_refresh_at_ms,
            last_live_state_refresh_at_ms: self.last_live_state_refresh_at_ms,
            last_event_at_ms: self.last_event_at_ms,
            last_successful_command_at_ms: self.last_successful_command_at_ms,
            last_successful_bridge_roundtrip_at_ms: self.last_successful_bridge_roundtrip_at_ms,
        }
    }

    pub fn current_url(&self) -> Option<&str> {
        self.current_url.as_deref()
    }

    fn record_trace(
        &mut self,
        request: BatchRequest,
        response: BatchResponse,
        emitted_events: &[SequencedEvent],
    ) -> Result<(), AegisError> {
        if let Some(recorder) = &mut self.trace_recorder {
            recorder.record_batch(request, response, emitted_events);
            recorder.flush()?;
        }
        Ok(())
    }

    fn ensure_runtime_bootstrapped(&mut self, capture_snapshot: bool) -> Result<(), AegisError> {
        if capture_snapshot && !self.dom_snapshot_valid {
            self.refresh_dom_snapshot()?;
        }
        Ok(())
    }

    fn commands_require_dom_snapshot(&self, commands: &[Command]) -> bool {
        commands.iter().any(|command| {
            matches!(
                command,
                Command::Click { .. }
                    | Command::Hover { .. }
                    | Command::SetValue { .. }
                    | Command::PressKey {
                        target: Some(_),
                        ..
                    }
                    | Command::WaitFor {
                        target: Some(_),
                        ..
                    }
            )
        })
    }

    fn refresh_dom_snapshot(&mut self) -> Result<(), AegisError> {
        let _ = self.drain_pending_events()?;
        let snapshot = self.bridge.snapshot_dom()?;
        self.dom.replace_snapshot(snapshot);
        self.dom_snapshot_valid = true;
        self.last_dom_refresh_at_ms = Some(now_ms());
        self.mark_successful_bridge_roundtrip();
        let _ = self.refresh_live_state(false);
        Ok(())
    }

    fn execute_command_stream(
        &mut self,
        batch_id: u64,
        commands: &[Command],
    ) -> Result<(BatchResponse, Vec<CommandResult>, Vec<SequencedEvent>), AegisError> {
        let mut pending = Vec::new();
        let mut results = Vec::new();
        let mut all_events = Vec::new();
        let mut final_snapshot = None;

        for command in commands {
            if matches!(command, Command::WaitFor { .. }) {
                let (batch_results, batch_events, _snapshot) =
                    self.flush_pending_commands(batch_id, &pending)?;
                results.extend(batch_results);
                all_events.extend(batch_events);
                pending.clear();

                let wait_result = self.execute_wait_for(command)?;
                results.push(wait_result);
                final_snapshot = Some(self.dom.snapshot());
            } else {
                pending.push(command.clone());
            }
        }

        let (batch_results, batch_events, snapshot) =
            self.flush_pending_commands(batch_id, &pending)?;
        results.extend(batch_results);
        all_events.extend(batch_events);
        if let Some(snapshot) = snapshot {
            final_snapshot = Some(snapshot);
        }

        Ok((
            BatchResponse {
                batch_id,
                results: results.clone(),
                snapshot: final_snapshot,
                events: all_events
                    .iter()
                    .map(|event| BridgeEventEnvelope {
                        event: event.event.clone(),
                    })
                    .collect(),
            },
            results,
            all_events,
        ))
    }

    fn flush_pending_commands(
        &mut self,
        batch_id: u64,
        commands: &[Command],
    ) -> Result<PendingBatchFlush, AegisError> {
        if commands.is_empty() {
            return Ok((Vec::new(), Vec::new(), None));
        }

        let mut results = Vec::new();
        let mut all_events = Vec::new();
        let mut final_snapshot = None;

        for command in commands {
            if self.command_target_needs_fresh_snapshot(command)
                && let Err(error) = self.refresh_dom_snapshot()
            {
                results.push(CommandResult::err(error.to_string()));
                continue;
            }
            let resolved = match self.resolve_command_for_bridge(command) {
                Ok(command) => command,
                Err(error) => {
                    results.push(error);
                    continue;
                }
            };

            let request = BatchRequest {
                batch_id,
                commands: vec![resolved],
            };
            let response = self.bridge.send_batch(&request)?;
            results.extend(response.results.clone());
            final_snapshot = response.snapshot.clone().or(final_snapshot);
            let emitted_events = self.apply_response(response)?;
            all_events.extend(emitted_events);
            let _ = self.refresh_live_state(true);
        }

        Ok((results, all_events, final_snapshot))
    }

    fn command_target_needs_fresh_snapshot(&self, command: &Command) -> bool {
        matches!(
            command,
            Command::Click {
                target: CommandTarget::Match { .. }
            } | Command::Hover {
                target: CommandTarget::Match { .. }
            } | Command::SetValue {
                target: CommandTarget::Match { .. },
                ..
            } | Command::PressKey {
                target: Some(CommandTarget::Match { .. }),
                ..
            }
        )
    }

    fn resolve_command_for_bridge(&self, command: &Command) -> Result<Command, CommandResult> {
        let snapshot = self.dom.snapshot();
        match command {
            Command::Click { target } => Ok(Command::Click {
                target: self.resolve_target_id(&snapshot, target, Some(DesiredAction::Click))?,
            }),
            Command::Hover { target } => Ok(Command::Hover {
                target: self.resolve_target_id(&snapshot, target, Some(DesiredAction::Hover))?,
            }),
            Command::SetValue { target, value } => Ok(Command::SetValue {
                target: self.resolve_target_id(&snapshot, target, Some(DesiredAction::Type))?,
                value: value.clone(),
            }),
            Command::PressKey {
                target,
                key,
                code,
                alt_key,
                ctrl_key,
                meta_key,
                shift_key,
            } => Ok(Command::PressKey {
                target: target
                    .as_ref()
                    .map(|target| {
                        self.resolve_target_id(&snapshot, target, Some(DesiredAction::PressKey))
                    })
                    .transpose()?,
                key: key.clone(),
                code: code.clone(),
                alt_key: *alt_key,
                ctrl_key: *ctrl_key,
                meta_key: *meta_key,
                shift_key: *shift_key,
            }),
            _ => Ok(command.clone()),
        }
    }

    fn resolve_target_id(
        &self,
        snapshot: &DomSnapshot,
        target: &CommandTarget,
        action: Option<DesiredAction>,
    ) -> Result<CommandTarget, CommandResult> {
        match target {
            CommandTarget::Id { .. } => Ok(target.clone()),
            CommandTarget::Match { matcher } => resolve_command_target(snapshot, target, action)
                .map(|node| CommandTarget::Id { id: node.id })
                .ok_or_else(|| CommandResult::err(format!("no node matched {}", json!(matcher)))),
        }
    }

    fn execute_wait_for(&mut self, command: &Command) -> Result<CommandResult, AegisError> {
        let Command::WaitFor {
            target,
            url_contains,
            title_contains,
            text,
            ready_state,
            timeout_ms,
            poll_interval_ms,
        } = command
        else {
            unreachable!("wait_for command required");
        };

        let timeout_ms = timeout_ms.unwrap_or(DEFAULT_WAIT_TIMEOUT_MS);
        let poll_interval_ms = poll_interval_ms
            .unwrap_or(DEFAULT_WAIT_POLL_INTERVAL_MS)
            .max(MIN_WAIT_POLL_INTERVAL_MS);
        let deadline = now_ms().saturating_add(timeout_ms);

        loop {
            let _ = self.bridge.pump();
            let _ = self.drain_pending_events();
            let _ = self.refresh_live_state(true);

            if self.wait_condition_satisfied(
                target.as_ref(),
                url_contains.as_deref(),
                title_contains.as_deref(),
                text.as_deref(),
                ready_state.as_deref(),
            )? {
                return Ok(CommandResult::ok(json!({
                    "ok": true,
                    "waited_ms": timeout_ms.saturating_sub(deadline.saturating_sub(now_ms())),
                    "current_url": self.current_url.clone(),
                    "current_title": self.current_title.clone(),
                    "document_ready_state": self.document_ready_state.clone()
                })));
            }

            if now_ms() >= deadline {
                return Ok(CommandResult::err("wait_for timed out"));
            }

            thread::sleep(Duration::from_millis(poll_interval_ms));
        }
    }

    fn wait_condition_satisfied(
        &mut self,
        target: Option<&CommandTarget>,
        url_contains: Option<&str>,
        title_contains: Option<&str>,
        text: Option<&str>,
        ready_state: Option<&str>,
    ) -> Result<bool, AegisError> {
        if url_contains.is_some_and(|needle| {
            !includes_normalized(self.current_url.as_deref().unwrap_or_default(), needle)
        }) {
            return Ok(false);
        }
        if title_contains.is_some_and(|needle| {
            !includes_normalized(self.current_title.as_deref().unwrap_or_default(), needle)
        }) {
            return Ok(false);
        }
        if ready_state.is_some_and(|expected| {
            !includes_normalized(
                self.document_ready_state.as_deref().unwrap_or_default(),
                expected,
            )
        }) {
            return Ok(false);
        }

        if target.is_some() || text.is_some() {
            self.refresh_dom_snapshot()?;
        }
        if let Some(target) = target
            && resolve_command_target(&self.dom.snapshot(), target, None).is_none()
        {
            return Ok(false);
        }
        if let Some(needle) = text
            && !self
                .dom
                .snapshot()
                .nodes
                .iter()
                .any(|node| includes_normalized(node.text.as_deref().unwrap_or_default(), needle))
        {
            return Ok(false);
        }

        Ok(true)
    }

    fn refresh_live_state(&mut self, force: bool) -> Result<(), AegisError> {
        if !force
            && self
                .last_live_state_refresh_at_ms
                .is_some_and(|last| now_ms().saturating_sub(last) < LIVE_STATE_REFRESH_INTERVAL_MS)
        {
            return Ok(());
        }

        let _ = self.ensure_page_telemetry_probe();
        let script = r#"JSON.stringify({
            url: window.location ? window.location.href : null,
            title: document.title || null,
            readyState: document.readyState || null
        })"#;
        let raw = self.bridge.eval_js(script)?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|error| AegisError::Bridge(format!("live state json parse error: {error}")))?;
        self.current_url = value
            .get("url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| self.current_url.clone());
        self.current_title = value
            .get("title")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        self.document_ready_state = value
            .get("readyState")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        self.last_live_state_refresh_at_ms = Some(now_ms());
        self.mark_successful_bridge_roundtrip();
        Ok(())
    }

    fn ensure_page_telemetry_probe(&mut self) -> Result<(), AegisError> {
        let script = r#"(() => {
            const telemetry = window.__aegisTelemetry = window.__aegisTelemetry || {
              installedAtMs: Date.now(),
              observersInstalled: false,
              cumulativeLayoutShift: 0,
              layoutShiftCount: 0,
              largestContentfulPaintMs: null,
              largestContentfulPaintSize: null,
              longTaskCount: 0,
              longTaskTotalDurationMs: 0,
              longTaskMaxDurationMs: 0,
              eventCount: 0,
              interactionCount: 0,
              totalEventDurationMs: 0,
              maxEventDurationMs: 0,
              firstInputDelayMs: null,
              interactionIds: {}
            };
            if (telemetry.observersInstalled) {
              return "ok";
            }

            const observe = (type, handler) => {
              try {
                const observer = new PerformanceObserver((list) => handler(list.getEntries()));
                observer.observe({ type, buffered: true });
                return true;
              } catch (_error) {
                return false;
              }
            };

            observe("largest-contentful-paint", (entries) => {
              for (const entry of entries) {
                if (Number.isFinite(entry.startTime)) {
                  telemetry.largestContentfulPaintMs = entry.startTime;
                }
                if (Number.isFinite(entry.size)) {
                  telemetry.largestContentfulPaintSize = entry.size;
                }
              }
            });

            observe("layout-shift", (entries) => {
              for (const entry of entries) {
                if (entry.hadRecentInput) {
                  continue;
                }
                if (Number.isFinite(entry.value)) {
                  telemetry.cumulativeLayoutShift += entry.value;
                  telemetry.layoutShiftCount += 1;
                }
              }
            });

            observe("longtask", (entries) => {
              for (const entry of entries) {
                const duration = Number(entry.duration ?? 0);
                if (!Number.isFinite(duration) || duration < 0) {
                  continue;
                }
                telemetry.longTaskCount += 1;
                telemetry.longTaskTotalDurationMs += duration;
                telemetry.longTaskMaxDurationMs = Math.max(telemetry.longTaskMaxDurationMs, duration);
              }
            });

            observe("event", (entries) => {
              for (const entry of entries) {
                const duration = Number(entry.duration ?? 0);
                if (Number.isFinite(duration) && duration >= 0) {
                  telemetry.eventCount += 1;
                  telemetry.totalEventDurationMs += duration;
                  telemetry.maxEventDurationMs = Math.max(telemetry.maxEventDurationMs, duration);
                }
                if (Number.isFinite(entry.interactionId) && entry.interactionId > 0) {
                  const key = String(entry.interactionId);
                  if (!telemetry.interactionIds[key]) {
                    telemetry.interactionIds[key] = true;
                    telemetry.interactionCount += 1;
                  }
                }
              }
            });

            observe("first-input", (entries) => {
              for (const entry of entries) {
                const delay = Number(entry.processingStart ?? 0) - Number(entry.startTime ?? 0);
                if (!Number.isFinite(delay) || delay < 0) {
                  continue;
                }
                if (telemetry.firstInputDelayMs == null || delay < telemetry.firstInputDelayMs) {
                  telemetry.firstInputDelayMs = delay;
                }
              }
            });

            telemetry.observersInstalled = true;
            return "installed";
        })()"#;
        let _ = self.bridge.eval_js(script)?;
        self.mark_successful_bridge_roundtrip();
        Ok(())
    }

    fn capture_page_runtime_telemetry(&mut self) -> Result<PageRuntimeTelemetry, AegisError> {
        let _ = self.ensure_page_telemetry_probe();
        let script = r#"(() => {
            const nav = performance.getEntriesByType("navigation")[0];
            const resources = performance.getEntriesByType("resource");
            const paints = performance.getEntriesByType("paint");
            const initiatorTypes = ["script", "link", "img", "fetch", "xmlhttprequest"];
            const summarize = (name) => resources.filter((entry) => entry.initiatorType === name).length;
            const sum = (key) => {
              const values = resources
                .map((entry) => Number(entry[key] ?? 0))
                .filter((value) => Number.isFinite(value) && value >= 0);
              if (values.length === 0) {
                return null;
              }
              return values.reduce((acc, value) => acc + value, 0);
            };
            const memory = performance && performance.memory ? performance.memory : null;
            const pageTelemetry = window.__aegisTelemetry || null;
            const paintValue = (name) => {
              const entry = paints.find((candidate) => candidate.name === name);
              return entry && Number.isFinite(entry.startTime) ? entry.startTime : null;
            };
            return JSON.stringify({
              sampledAtMs: Date.now(),
              url: window.location ? window.location.href : null,
              title: document && document.title ? document.title : null,
              readyState: document && document.readyState ? document.readyState : null,
              origin: window.location ? window.location.origin : null,
              visibilityState: document && document.visibilityState ? document.visibilityState : null,
              hasFocus: document && typeof document.hasFocus === "function" ? document.hasFocus() : null,
              viewport: {
                width: typeof window.innerWidth === "number" ? window.innerWidth : null,
                height: typeof window.innerHeight === "number" ? window.innerHeight : null,
                devicePixelRatio: typeof window.devicePixelRatio === "number" ? window.devicePixelRatio : null,
                scrollX: typeof window.scrollX === "number" ? window.scrollX : null,
                scrollY: typeof window.scrollY === "number" ? window.scrollY : null
              },
              navigation: nav ? {
                navigationType: nav.type ?? null,
                domContentLoadedMs: Number.isFinite(nav.domContentLoadedEventEnd) ? nav.domContentLoadedEventEnd : null,
                loadEventMs: Number.isFinite(nav.loadEventEnd) ? nav.loadEventEnd : null,
                domInteractiveMs: Number.isFinite(nav.domInteractive) ? nav.domInteractive : null,
                responseEndMs: Number.isFinite(nav.responseEnd) ? nav.responseEnd : null,
                transferSizeBytes: Number.isFinite(nav.transferSize) ? nav.transferSize : null,
                encodedBodySizeBytes: Number.isFinite(nav.encodedBodySize) ? nav.encodedBodySize : null,
                decodedBodySizeBytes: Number.isFinite(nav.decodedBodySize) ? nav.decodedBodySize : null
              } : null,
              resources: {
                resourceCount: resources.length,
                scriptCount: summarize("script"),
                stylesheetCount: summarize("link"),
                imageCount: summarize("img"),
                fetchCount: summarize("fetch"),
                xmlHttpRequestCount: summarize("xmlhttprequest"),
                otherCount: resources.filter((entry) => !initiatorTypes.includes(entry.initiatorType)).length,
                transferSizeBytes: sum("transferSize"),
                encodedBodySizeBytes: sum("encodedBodySize"),
                decodedBodySizeBytes: sum("decodedBodySize")
              },
              jsHeap: memory ? {
                usedJsHeapSizeBytes: Number.isFinite(memory.usedJSHeapSize) ? memory.usedJSHeapSize : null,
                totalJsHeapSizeBytes: Number.isFinite(memory.totalJSHeapSize) ? memory.totalJSHeapSize : null,
                jsHeapSizeLimitBytes: Number.isFinite(memory.jsHeapSizeLimit) ? memory.jsHeapSizeLimit : null
              } : null,
              paint: {
                firstPaintMs: paintValue("first-paint"),
                firstContentfulPaintMs: paintValue("first-contentful-paint"),
                largestContentfulPaintMs: pageTelemetry && Number.isFinite(pageTelemetry.largestContentfulPaintMs)
                  ? pageTelemetry.largestContentfulPaintMs
                  : null,
                largestContentfulPaintSize: pageTelemetry && Number.isFinite(pageTelemetry.largestContentfulPaintSize)
                  ? pageTelemetry.largestContentfulPaintSize
                  : null
              },
              stability: {
                cumulativeLayoutShift: pageTelemetry && Number.isFinite(pageTelemetry.cumulativeLayoutShift)
                  ? pageTelemetry.cumulativeLayoutShift
                  : null,
                layoutShiftCount: pageTelemetry && Number.isFinite(pageTelemetry.layoutShiftCount)
                  ? pageTelemetry.layoutShiftCount
                  : 0
              },
              responsiveness: {
                longTaskCount: pageTelemetry && Number.isFinite(pageTelemetry.longTaskCount)
                  ? pageTelemetry.longTaskCount
                  : 0,
                longTaskTotalDurationMs: pageTelemetry && Number.isFinite(pageTelemetry.longTaskTotalDurationMs)
                  ? pageTelemetry.longTaskTotalDurationMs
                  : null,
                longTaskMaxDurationMs: pageTelemetry && Number.isFinite(pageTelemetry.longTaskMaxDurationMs)
                  ? pageTelemetry.longTaskMaxDurationMs
                  : null,
                eventCount: pageTelemetry && Number.isFinite(pageTelemetry.eventCount)
                  ? pageTelemetry.eventCount
                  : 0,
                interactionCount: pageTelemetry && Number.isFinite(pageTelemetry.interactionCount)
                  ? pageTelemetry.interactionCount
                  : 0,
                totalEventDurationMs: pageTelemetry && Number.isFinite(pageTelemetry.totalEventDurationMs)
                  ? pageTelemetry.totalEventDurationMs
                  : null,
                maxEventDurationMs: pageTelemetry && Number.isFinite(pageTelemetry.maxEventDurationMs)
                  ? pageTelemetry.maxEventDurationMs
                  : null,
                firstInputDelayMs: pageTelemetry && Number.isFinite(pageTelemetry.firstInputDelayMs)
                  ? pageTelemetry.firstInputDelayMs
                  : null
              }
            });
        })()"#;
        let raw = self.bridge.eval_js(script)?;
        let value: Value = serde_json::from_str(&raw).map_err(|error| {
            AegisError::Bridge(format!("page telemetry json parse error: {error}"))
        })?;
        self.mark_successful_bridge_roundtrip();
        Ok(PageRuntimeTelemetry {
            sampled_at_ms: value
                .get("sampledAtMs")
                .and_then(Value::as_u64)
                .unwrap_or_else(now_ms),
            url: value.get("url").and_then(Value::as_str).map(ToOwned::to_owned),
            title: value
                .get("title")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            ready_state: value
                .get("readyState")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            origin: value
                .get("origin")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            visibility_state: value
                .get("visibilityState")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            has_focus: value.get("hasFocus").and_then(Value::as_bool),
            viewport: serde_json::from_value(
                value.get("viewport").cloned().unwrap_or_else(|| json!({})),
            )
            .unwrap_or_default(),
            navigation: serde_json::from_value(
                value.get("navigation").cloned().unwrap_or_else(|| json!({})),
            )
            .unwrap_or_default(),
            resources: serde_json::from_value(
                value.get("resources").cloned().unwrap_or_else(|| json!({})),
            )
            .unwrap_or_default(),
            js_heap: serde_json::from_value(
                value.get("jsHeap").cloned().unwrap_or_else(|| json!({})),
            )
            .unwrap_or_default(),
            paint: serde_json::from_value(
                value.get("paint").cloned().unwrap_or_else(|| json!({})),
            )
            .unwrap_or_default(),
            stability: serde_json::from_value(
                value.get("stability").cloned().unwrap_or_else(|| json!({})),
            )
            .unwrap_or_default(),
            responsiveness: serde_json::from_value(
                value.get("responsiveness")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            )
            .unwrap_or_default(),
        })
    }

    fn build_dom_telemetry(&self) -> DomTelemetrySummary {
        let snapshot = self.dom.snapshot();
        let mut actionable_nodes = 0;
        let mut visible_nodes = 0;
        let mut disabled_nodes = 0;
        let mut text_nodes = 0;
        for node in &snapshot.nodes {
            if node.text.as_ref().is_some_and(|text| !text.trim().is_empty()) {
                text_nodes += 1;
            }
            if let Some(semantic) = node.semantic.as_ref() {
                if semantic.actionable {
                    actionable_nodes += 1;
                }
                if semantic.visible {
                    visible_nodes += 1;
                }
                if semantic.disabled {
                    disabled_nodes += 1;
                }
            }
        }
        DomTelemetrySummary {
            total_nodes: snapshot.nodes.len(),
            actionable_nodes,
            visible_nodes,
            disabled_nodes,
            text_nodes,
        }
    }

    fn trace_telemetry(&self) -> TraceTelemetry {
        match self.trace_recorder.as_ref() {
            Some(recorder) => TraceTelemetry {
                enabled: true,
                path: Some(recorder.path().display().to_string()),
                recorded_batches: recorder.batch_count(),
                initial_session_captured: recorder.has_initial_session(),
                file_size_bytes: std::fs::metadata(recorder.path()).ok().map(|meta| meta.len()),
            },
            None => TraceTelemetry::default(),
        }
    }

    fn network_summary(&self) -> NetworkTelemetrySummary {
        let mut top_domains = self
            .network_domains
            .iter()
            .map(|(host, aggregate)| NetworkDomainTelemetry {
                host: host.clone(),
                request_count: aggregate.request_count,
                failure_count: aggregate.failure_count,
                redirect_count: aggregate.redirect_count,
                transferred_bytes: aggregate.transferred_bytes,
                avg_duration_ms: if aggregate.duration_count > 0 {
                    Some(aggregate.total_duration_ms / aggregate.duration_count)
                } else {
                    None
                },
                max_duration_ms: if aggregate.duration_count > 0 {
                    Some(aggregate.max_duration_ms)
                } else {
                    None
                },
            })
            .collect::<Vec<_>>();
        top_domains.sort_by(|left, right| {
            right
                .transferred_bytes
                .cmp(&left.transferred_bytes)
                .then_with(|| right.request_count.cmp(&left.request_count))
                .then_with(|| left.host.cmp(&right.host))
        });
        top_domains.truncate(5);

        let mut method_breakdown = breakdown_from_map(&self.network_methods);
        method_breakdown.truncate(6);

        let mut mime_breakdown = breakdown_from_map(&self.network_mime_types);
        mime_breakdown.truncate(6);

        let mut status_code_breakdown = breakdown_from_map(&self.network_status_codes);
        status_code_breakdown.truncate(8);

        let mut top_errors = breakdown_from_map(&self.network_errors);
        top_errors.truncate(6);

        NetworkTelemetrySummary {
            total_requests: self.network_events,
            successful_requests: self.successful_network_requests,
            failed_requests: self.failed_network_requests,
            redirected_requests: self.redirected_network_requests,
            main_frame_requests: self.main_frame_network_requests,
            informational_responses: self.informational_responses,
            successful_responses: self.successful_responses,
            redirect_responses: self.redirect_responses,
            client_error_responses: self.client_error_responses,
            server_error_responses: self.server_error_responses,
            transferred_bytes: self.transferred_network_bytes,
            avg_duration_ms: if self.network_duration_samples > 0 {
                Some(self.total_network_duration_ms / self.network_duration_samples)
            } else {
                None
            },
            max_duration_ms: if self.network_duration_samples > 0 {
                Some(self.max_network_duration_ms)
            } else {
                None
            },
            method_breakdown,
            mime_breakdown,
            status_code_breakdown,
            top_errors,
            top_domains,
        }
    }

    fn mark_successful_bridge_roundtrip(&mut self) {
        self.last_successful_bridge_roundtrip_at_ms = Some(now_ms());
    }

    fn mark_successful_command(&mut self) {
        let now = now_ms();
        self.last_successful_bridge_roundtrip_at_ms = Some(now);
        self.last_successful_command_at_ms = Some(now);
    }
}

fn includes_normalized(haystack: &str, needle: &str) -> bool {
    normalize_text(haystack).contains(&normalize_text(needle))
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn push_recent<T>(queue: &mut VecDeque<T>, value: T) {
    queue.push_back(value);
    while queue.len() > RECENT_TELEMETRY_LIMIT {
        let _ = queue.pop_front();
    }
}

fn extract_host(url: &str) -> Option<String> {
    let (_, remainder) = url.split_once("://")?;
    let authority = remainder.split('/').next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host).trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

fn normalize_mime_group(mime_type: &str) -> String {
    let mime_type = mime_type.trim().to_ascii_lowercase();
    if mime_type.is_empty() {
        return "unknown".into();
    }
    if let Some((group, subtype)) = mime_type.split_once('/') {
        let subtype = subtype.split(';').next().unwrap_or(subtype).trim();
        return format!("{group}/{subtype}");
    }
    mime_type
}

fn build_network_error_key(
    request_status: Option<&str>,
    error_code: Option<&i32>,
    error_text: Option<&str>,
) -> Option<String> {
    if let Some(error_code) = error_code {
        let mut key = format!("code:{error_code}");
        if let Some(error_text) = error_text.map(str::trim).filter(|value| !value.is_empty()) {
            key.push(' ');
            key.push_str(error_text);
        }
        return Some(key);
    }
    if let Some(error_text) = error_text.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(error_text.to_string());
    }
    request_status
        .map(str::trim)
        .filter(|value| matches!(*value, "failed" | "canceled"))
        .map(ToOwned::to_owned)
}

fn breakdown_from_map(
    aggregates: &BTreeMap<String, NetworkBreakdownAggregate>,
) -> Vec<NetworkBreakdownTelemetry> {
    let mut breakdown = aggregates
        .iter()
        .map(|(key, aggregate)| NetworkBreakdownTelemetry {
            key: key.clone(),
            count: aggregate.count,
        })
        .collect::<Vec<_>>();
    breakdown.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.key.cmp(&right.key))
    });
    breakdown
}
