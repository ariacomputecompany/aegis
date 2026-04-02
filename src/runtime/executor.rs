use crate::browser::BrowserConfig;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::VecDeque;
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
    pub recent_navigations: Vec<RecentNavigationTelemetry>,
    pub recent_network_requests: Vec<RecentNetworkRequestTelemetry>,
    pub recent_logs: Vec<RecentLogTelemetry>,
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
            } => {
                self.network_events += 1;
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
