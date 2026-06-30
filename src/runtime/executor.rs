use crate::browser::BrowserConfig;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::commands::command::{Command, CommandResult, CommandTarget, UploadFilePayload};
use crate::dom::diff::DomMutation;
use crate::dom::node::DomSnapshot;
use crate::dom::tree::DomTree;
use crate::events::stream::{EventReadWindow, EventStream, RuntimeEvent, SequencedEvent};
use crate::runtime::scheduler::Scheduler;
use crate::session::cookies::SessionState;
use crate::trace::recorder::TraceRecorder;
use crate::transfers::stage_upload_file;
use crate::transport::bridge::{
    AegisError, BatchRequest, BatchResponse, BridgeEventEnvelope, CefBridge,
};
use crate::transport::protocol::HostRuntimeState;

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
    #[serde(default)]
    pub page_bootstrap: PageBootstrapDiagnostics,
    #[serde(default)]
    pub media: Vec<MediaDiagnostics>,
    pub last_dom_refresh_at_ms: Option<u64>,
    pub last_live_state_refresh_at_ms: Option<u64>,
    pub last_event_at_ms: Option<u64>,
    pub last_successful_command_at_ms: Option<u64>,
    pub last_successful_bridge_roundtrip_at_ms: Option<u64>,
    pub host: HostRuntimeState,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageBootstrapDiagnostics {
    pub document_loaded: bool,
    pub document_loaded_at_ms: Option<u64>,
    pub module_scripts_present: bool,
    pub module_script_count: usize,
    #[serde(default)]
    pub module_script_sources: Vec<String>,
    pub synthetic_shell_active: bool,
    pub root_selector: Option<String>,
    pub root_present: bool,
    pub root_child_element_count: usize,
    pub root_text_length: usize,
    pub root_html_length: usize,
    pub body_text_length: usize,
    pub body_descendant_count: usize,
    pub dom_mutation_count: usize,
    pub app_dom_mutated_after_load: bool,
    pub body_mutation_after_load_count: usize,
    pub root_mutation_after_load_count: usize,
    pub module_bootstrap_observed: bool,
    pub inspectable_dom_ready: bool,
    pub script_error_count: usize,
    pub last_script_error: Option<String>,
    pub unhandled_rejection_count: usize,
    pub last_unhandled_rejection: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaDiagnostics {
    pub index: usize,
    pub node_id: Option<u64>,
    pub tag: String,
    pub current_src: Option<String>,
    #[serde(default)]
    pub source_codec_support: MediaCodecSupport,
    pub ready_state: Option<u8>,
    pub network_state: Option<u8>,
    pub duration: Option<f64>,
    pub paused: Option<bool>,
    pub ended: Option<bool>,
    pub muted: Option<bool>,
    pub seeking: Option<bool>,
    pub current_time: Option<f64>,
    pub playback_rate: Option<f64>,
    pub volume: Option<f64>,
    pub loop_enabled: Option<bool>,
    pub autoplay: Option<bool>,
    pub controls: Option<bool>,
    pub play_attempts: Option<u64>,
    pub play_resolved: Option<u64>,
    pub play_rejected: Option<u64>,
    pub pause_calls: Option<u64>,
    pub load_calls: Option<u64>,
    pub loaded_metadata_count: Option<u64>,
    pub metadata_parse_attempted: Option<bool>,
    pub stalled_count: Option<u64>,
    pub last_event: Option<String>,
    #[serde(default)]
    pub recent_events: Vec<String>,
    #[serde(default)]
    pub event_timeline: Vec<MediaEventSnapshot>,
    pub resource_timing: Option<MediaResourceTiming>,
    pub error: Option<String>,
    pub error_code: Option<u8>,
    pub error_message: Option<String>,
    pub likely_failure_cause: Option<String>,
    pub last_play_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaCodecSupport {
    pub audio_mp4: Option<String>,
    pub audio_mp4_aac_lc: Option<String>,
    pub audio_aac: Option<String>,
    pub audio_mpeg: Option<String>,
    pub audio_ogg_opus: Option<String>,
    pub audio_wav_pcm: Option<String>,
    pub video_mp4_h264_aac: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaEventSnapshot {
    pub event: String,
    pub at_ms: Option<u64>,
    pub ready_state: Option<u8>,
    pub network_state: Option<u8>,
    pub paused: Option<bool>,
    pub current_time: Option<f64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MediaResourceTiming {
    pub initiator_type: Option<String>,
    pub transfer_size: Option<u64>,
    pub encoded_body_size: Option<u64>,
    pub decoded_body_size: Option<u64>,
    pub duration_ms: Option<f64>,
    pub response_end_ms: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WaitLiveState {
    scroll_x: Option<i64>,
    scroll_y: Option<i64>,
    selector_found: bool,
    animations_running: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageResearchHeading {
    pub level: Option<u8>,
    pub text: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageResearchLink {
    pub index: usize,
    pub text: String,
    pub href: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageContentScopes {
    #[serde(default)]
    pub main_text: String,
    #[serde(default)]
    pub article_text: String,
    #[serde(default)]
    pub controls_text: String,
    #[serde(default)]
    pub overlay_text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageResearchControl {
    pub index: usize,
    pub kind: String,
    pub role: Option<String>,
    pub text: String,
    pub label: Option<String>,
    pub placeholder: Option<String>,
    pub control_type: Option<String>,
    pub href: Option<String>,
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub in_main_content: bool,
    #[serde(default)]
    pub in_viewport: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageResearchForm {
    pub index: usize,
    pub id: Option<String>,
    pub name: Option<String>,
    pub action: Option<String>,
    pub method: Option<String>,
    #[serde(default)]
    pub controls: Vec<PageResearchControl>,
    #[serde(default)]
    pub submit_labels: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageResearchOverlay {
    pub index: usize,
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default)]
    pub dismiss_labels: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageSemanticSummary {
    #[serde(default)]
    pub link_count: usize,
    #[serde(default)]
    pub control_count: usize,
    #[serde(default)]
    pub form_count: usize,
    #[serde(default)]
    pub overlay_count: usize,
    #[serde(default)]
    pub dialog_count: usize,
    #[serde(default)]
    pub alert_count: usize,
    #[serde(default)]
    pub code_block_count: usize,
    #[serde(default)]
    pub table_count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PageResearchData {
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub visible_text: String,
    #[serde(default)]
    pub content_scopes: PageContentScopes,
    pub main_content_selector: Option<String>,
    #[serde(default)]
    pub headings: Vec<PageResearchHeading>,
    #[serde(default)]
    pub links: Vec<PageResearchLink>,
    #[serde(default)]
    pub primary_links: Vec<PageResearchLink>,
    #[serde(default)]
    pub controls: Vec<PageResearchControl>,
    #[serde(default)]
    pub primary_controls: Vec<PageResearchControl>,
    #[serde(default)]
    pub forms: Vec<PageResearchForm>,
    #[serde(default)]
    pub overlays: Vec<PageResearchOverlay>,
    #[serde(default)]
    pub semantic_summary: PageSemanticSummary,
    #[serde(default)]
    pub page_type: String,
    #[serde(default)]
    pub useful_text_available: bool,
    #[serde(default)]
    pub interactive_elements_available: bool,
    #[serde(default)]
    pub auth_wall_likely: bool,
    #[serde(default)]
    pub blocked_by_overlay: bool,
    #[serde(default)]
    pub blocker_signals: Vec<String>,
    #[serde(default)]
    pub suggested_next_actions: Vec<String>,
    #[serde(default)]
    pub likely_not_found: bool,
    #[serde(default)]
    pub not_found_signals: Vec<String>,
    pub suggested_search_query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionReport {
    pub batch_id: u64,
    pub results: Vec<CommandResult>,
    pub latest_event_sequence: u64,
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
    media: Vec<MediaDiagnostics>,
    page_bootstrap: PageBootstrapDiagnostics,
    last_dom_refresh_at_ms: Option<u64>,
    last_live_state_refresh_at_ms: Option<u64>,
    last_event_at_ms: Option<u64>,
    last_successful_command_at_ms: Option<u64>,
    last_successful_bridge_roundtrip_at_ms: Option<u64>,
    host_state: HostRuntimeState,
    network_event_capture_enabled: bool,
}

const LIVE_STATE_REFRESH_INTERVAL_MS: u64 = 250;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_WAIT_POLL_INTERVAL_MS: u64 = 50;
const MIN_WAIT_POLL_INTERVAL_MS: u64 = 10;

type PendingBatchFlush = (Vec<CommandResult>, Vec<SequencedEvent>, Option<DomSnapshot>);

#[derive(Debug, Clone, Copy, Default)]
struct CommandEventSpan {
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct WaitEventSummary {
    dom_changed: bool,
    navigation_changed: bool,
    network_changed: bool,
    log_changed: bool,
    media_signal_changed: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct LiveResolvedTarget {
    target_id: u64,
}

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
            page_bootstrap: PageBootstrapDiagnostics::default(),
            media: Vec::new(),
            last_dom_refresh_at_ms: None,
            last_live_state_refresh_at_ms: None,
            last_event_at_ms: None,
            last_successful_command_at_ms: None,
            last_successful_bridge_roundtrip_at_ms: None,
            host_state: HostRuntimeState::default(),
            network_event_capture_enabled: false,
        })
    }

    pub fn execute(&mut self, commands: &[Command]) -> Result<ExecutionReport, AegisError> {
        if commands_need_interaction_diagnostics(commands) {
            self.network_event_capture_enabled = true;
        }
        self.ensure_runtime_bootstrapped(false)?;
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
        let response = self
            .bridge
            .navigate(&url, self.network_event_capture_enabled)?;
        let request = BatchRequest {
            batch_id: self.scheduler.next_batch_id(),
            commands: Vec::new(),
        };
        let emitted_events = self.apply_response(response.clone())?;
        let _ = self.refresh_host_state();
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

    pub fn inject_session(&mut self, session: SessionState) -> Result<(), AegisError> {
        session.validate().map_err(AegisError::InvalidSession)?;
        if let Some(recorder) = &mut self.trace_recorder {
            recorder.set_initial_session(session.clone());
            recorder.flush()?;
        }
        self.bridge.inject_session(session)?;
        self.mark_successful_bridge_roundtrip();
        let _ = self.refresh_host_state();
        let _ = self.refresh_live_state(true);
        Ok(())
    }

    pub fn snapshot_session(&mut self) -> Result<SessionState, AegisError> {
        self.ensure_runtime_bootstrapped(false)?;
        let session = self.bridge.snapshot_session()?;
        self.mark_successful_bridge_roundtrip();
        let _ = self.refresh_host_state();
        let _ = self.refresh_live_state(false);
        Ok(session)
    }

    pub fn pump(&mut self) -> Result<(), AegisError> {
        self.bridge.pump()?;
        let _ = self.refresh_host_state();
        if self.host_state.runtime_ready {
            let _ = self.drain_pending_events();
            let _ = self.refresh_live_state(false);
        }
        Ok(())
    }

    pub fn establish_command_bridge(&mut self) -> Result<(), AegisError> {
        self.bridge.ensure_runtime()?;
        let raw_events = self
            .bridge
            .drain_events(self.network_event_capture_enabled)?;
        self.mark_successful_bridge_roundtrip();
        let _ = self.apply_event_batch(raw_events);
        let _ = self.refresh_host_state();
        let _ = self.refresh_live_state(true);
        Ok(())
    }

    pub fn snapshot_dom(&mut self) -> Result<crate::dom::node::DomSnapshot, AegisError> {
        self.refresh_dom_snapshot()?;
        Ok(self.dom.snapshot())
    }

    pub fn page_research_data(&mut self) -> Result<PageResearchData, AegisError> {
        self.refresh_live_state(true)?;
        let raw = self
            .bridge
            .eval_js("JSON.stringify(window.__aegis.pageResearchData())")?;
        serde_json::from_str(&raw)
            .map_err(|error| AegisError::Bridge(format!("page research json parse error: {error}")))
    }

    pub fn event_stream(&self) -> &EventStream {
        &self.events
    }

    pub fn drain_pending_events(&mut self) -> Result<Vec<SequencedEvent>, AegisError> {
        let raw_events = self
            .bridge
            .drain_events(self.network_event_capture_enabled)?;
        self.mark_successful_bridge_roundtrip();
        Ok(self.apply_event_batch(raw_events))
    }

    pub fn read_events_from(&self, sequence: u64) -> EventReadWindow {
        self.events.read_from(sequence, None)
    }

    pub fn bridge(&self) -> &CefBridge {
        &self.bridge
    }

    pub fn enable_trace_recording(&mut self, path: impl Into<std::path::PathBuf>) {
        self.network_event_capture_enabled = true;
        self.trace_recorder = Some(TraceRecorder::new(path, self.browser_config.clone()));
    }

    pub fn enable_network_event_capture(&mut self) {
        self.network_event_capture_enabled = true;
    }

    pub fn browser_config(&self) -> &BrowserConfig {
        &self.browser_config
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
            page_bootstrap: self.page_bootstrap.clone(),
            media: self.media.clone(),
            last_dom_refresh_at_ms: self.last_dom_refresh_at_ms,
            last_live_state_refresh_at_ms: self.last_live_state_refresh_at_ms,
            last_event_at_ms: self.last_event_at_ms,
            last_successful_command_at_ms: self.last_successful_command_at_ms,
            last_successful_bridge_roundtrip_at_ms: self.last_successful_bridge_roundtrip_at_ms,
            host: self.host_state.clone(),
        }
    }

    pub fn current_url(&self) -> Option<&str> {
        self.current_url.as_deref()
    }

    pub fn snapshot_host_state(&mut self) -> Result<HostRuntimeState, AegisError> {
        self.refresh_host_state()?;
        Ok(self.host_state.clone())
    }

    pub fn activate_browser(&mut self, browser_id: i32) -> Result<HostRuntimeState, AegisError> {
        self.ensure_runtime_bootstrapped(false)?;
        self.host_state = self.bridge.activate_browser(browser_id)?;
        if let Some(url) = self.host_state.current_url.clone() {
            self.current_url = Some(url);
        }
        self.mark_successful_bridge_roundtrip();
        let _ = self.refresh_live_state(true);
        Ok(self.host_state.clone())
    }

    pub fn request_cancel(&self) {
        self.bridge.request_cancel();
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
        self.refresh_host_state()?;
        if capture_snapshot && !self.dom_snapshot_valid {
            self.refresh_dom_snapshot()?;
        }
        Ok(())
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
            if matches!(
                command,
                Command::WaitFor { .. } | Command::MediaState { .. }
            ) {
                let (batch_results, batch_events, _snapshot) =
                    self.flush_pending_commands(batch_id, &pending)?;
                results.extend(batch_results);
                all_events.extend(batch_events);
                pending.clear();

                let command_result = match command {
                    Command::WaitFor { .. } => self.execute_wait_for(command)?,
                    Command::MediaState { target } => self.execute_media_state(target.as_ref())?,
                    _ => unreachable!("non-bridge command handled above"),
                };
                results.push(command_result);
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

        let mut results = vec![CommandResult::err("command not executed"); commands.len()];
        let mut prepared_commands = Vec::new();
        let mut prepared_positions = Vec::new();

        for (index, command) in commands.iter().enumerate() {
            match self.prepare_command_for_bridge(command) {
                Ok(prepared) => {
                    prepared_positions.push(index);
                    prepared_commands.push(prepared);
                }
                Err(error) => results[index] = error,
            }
        }

        if prepared_commands.is_empty() {
            return Ok((results, Vec::new(), None));
        }

        let request = BatchRequest {
            batch_id,
            commands: prepared_commands.clone(),
        };
        let response = self
            .bridge
            .send_batch(&request, self.network_event_capture_enabled)?;
        let response_results = response.results.clone();
        let final_snapshot = response.snapshot.clone();
        let emitted_events = self.apply_response(response)?;
        let previous_global_sequence = emitted_events
            .first()
            .map(|event| event.sequence.saturating_sub(1))
            .or_else(|| {
                self.events
                    .oldest_sequence()
                    .map(|_| self.events.latest_sequence())
            });

        for (bridge_index, mut result) in response_results.into_iter().enumerate() {
            let command_index = prepared_positions[bridge_index];
            let span = extract_command_event_span(&mut result.value);
            let command_events =
                command_events_for_span(&emitted_events, span, prepared_commands.len());
            let prior_event_sequence =
                command_prior_event_sequence(previous_global_sequence, &emitted_events, span);
            results[command_index] = self.annotate_command_result(
                &commands[command_index],
                result,
                prior_event_sequence,
                command_events,
            );
        }

        let _ = self.refresh_live_state(true);
        Ok((results, emitted_events, final_snapshot))
    }

    fn prepare_command_for_bridge(&self, command: &Command) -> Result<Command, CommandResult> {
        match command {
            Command::SetFiles { target, paths, .. } => Ok(Command::SetFiles {
                target: target.clone(),
                paths: paths.clone(),
                files: Some(load_upload_payloads(
                    paths,
                    self.browser_config.upload_dir.as_deref(),
                )?),
            }),
            _ => Ok(command.clone()),
        }
    }

    fn execute_wait_for(&mut self, command: &Command) -> Result<CommandResult, AegisError> {
        let Command::WaitFor {
            target,
            selector,
            url_contains,
            title_contains,
            text,
            ready_state,
            scroll_x,
            scroll_y,
            scroll_changed,
            media_current_src_contains,
            media_ready_state_at_least,
            media_duration_known,
            animation_idle_ms,
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
        let needs_live_wait = selector.is_some()
            || scroll_x.is_some()
            || scroll_y.is_some()
            || scroll_changed.unwrap_or(false)
            || animation_idle_ms.is_some();
        let needs_page_state = url_contains.is_some()
            || title_contains.is_some()
            || ready_state.is_some()
            || media_current_src_contains.is_some()
            || media_ready_state_at_least.is_some()
            || media_duration_known.is_some();
        let needs_dom = target.is_some() || text.is_some();
        let initial_scroll = if needs_live_wait {
            self.live_wait_state(selector.as_deref())?
        } else {
            WaitLiveState::default()
        };
        let mut animation_idle_since_ms = None;
        let mut dom_dirty = needs_dom;
        let mut live_state_dirty = needs_live_wait || needs_page_state;

        loop {
            let _ = self.bridge.pump();
            let drained_events = self.drain_pending_events().unwrap_or_default();
            let _ = self.refresh_host_state();
            let event_summary = WaitEventSummary::from_events(&drained_events);
            dom_dirty |= event_summary.dom_changed || event_summary.navigation_changed;
            live_state_dirty |= event_summary.navigation_changed
                || event_summary.dom_changed
                || event_summary.network_changed
                || event_summary.log_changed
                || event_summary.media_signal_changed;

            if live_state_dirty && (needs_live_wait || needs_page_state) {
                let _ = self.refresh_live_state(true);
                live_state_dirty = false;
            }
            if dom_dirty && needs_dom {
                let _ = self.refresh_dom_snapshot();
                dom_dirty = false;
            }

            if self.host_state.cancel_requested {
                return Ok(CommandResult::err("wait_for cancelled"));
            }

            if self.wait_condition_satisfied(
                target.as_ref(),
                selector.as_deref(),
                url_contains.as_deref(),
                title_contains.as_deref(),
                text.as_deref(),
                ready_state.as_deref(),
                *scroll_x,
                *scroll_y,
                *scroll_changed,
                media_current_src_contains.as_deref(),
                *media_ready_state_at_least,
                *media_duration_known,
                *animation_idle_ms,
                needs_live_wait.then_some(&initial_scroll),
                &mut animation_idle_since_ms,
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

    fn execute_media_state(
        &mut self,
        target: Option<&CommandTarget>,
    ) -> Result<CommandResult, AegisError> {
        self.refresh_live_state(true)?;
        let target_id = if let Some(target) = target {
            Some(self.resolve_live_target(target, None)?.target_id)
        } else {
            None
        };
        let media = self
            .media
            .iter()
            .filter(|entry| target_id.is_none_or(|id| entry.node_id == Some(id)))
            .cloned()
            .collect::<Vec<_>>();
        Ok(CommandResult::ok(json!({
            "count": media.len(),
            "media": media,
        })))
    }

    #[allow(clippy::too_many_arguments)]
    fn wait_condition_satisfied(
        &mut self,
        target: Option<&CommandTarget>,
        selector: Option<&str>,
        url_contains: Option<&str>,
        title_contains: Option<&str>,
        text: Option<&str>,
        ready_state: Option<&str>,
        scroll_x: Option<i64>,
        scroll_y: Option<i64>,
        scroll_changed: Option<bool>,
        media_current_src_contains: Option<&str>,
        media_ready_state_at_least: Option<u8>,
        media_duration_known: Option<bool>,
        animation_idle_ms: Option<u64>,
        initial_scroll: Option<&WaitLiveState>,
        animation_idle_since_ms: &mut Option<u64>,
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

        let live_state = if selector.is_some()
            || scroll_x.is_some()
            || scroll_y.is_some()
            || scroll_changed.unwrap_or(false)
            || animation_idle_ms.is_some()
        {
            Some(self.live_wait_state(selector)?)
        } else {
            None
        };

        if let Some(_selector) = selector
            && !live_state
                .as_ref()
                .is_some_and(|state| state.selector_found)
        {
            return Ok(false);
        }
        if scroll_x.is_some_and(|expected| {
            live_state.as_ref().and_then(|state| state.scroll_x) != Some(expected)
        }) {
            return Ok(false);
        }
        if scroll_y.is_some_and(|expected| {
            live_state.as_ref().and_then(|state| state.scroll_y) != Some(expected)
        }) {
            return Ok(false);
        }
        if scroll_changed.unwrap_or(false)
            && initial_scroll.is_some_and(|initial| {
                live_state.as_ref().is_some_and(|state| {
                    state.scroll_x == initial.scroll_x && state.scroll_y == initial.scroll_y
                })
            })
        {
            return Ok(false);
        }

        if let Some(target) = target
            && self.resolve_live_target(target, None).is_err()
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

        if let Some(needle) = media_current_src_contains
            && !self.media.iter().any(|media| {
                includes_normalized(media.current_src.as_deref().unwrap_or_default(), needle)
            })
        {
            return Ok(false);
        }
        if media_ready_state_at_least.is_some_and(|minimum| {
            !self
                .media
                .iter()
                .any(|media| media.ready_state.unwrap_or_default() >= minimum)
        }) {
            return Ok(false);
        }
        if media_duration_known.is_some_and(|required| {
            let has_duration = self.media.iter().any(|media| media.duration.is_some());
            has_duration != required
        }) {
            return Ok(false);
        }
        if let Some(idle_ms) = animation_idle_ms {
            let running = live_state
                .as_ref()
                .is_some_and(|state| state.animations_running);
            if running {
                *animation_idle_since_ms = None;
                return Ok(false);
            }
            let since = animation_idle_since_ms.get_or_insert_with(now_ms);
            if now_ms().saturating_sub(*since) < idle_ms {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn live_wait_state(&mut self, selector: Option<&str>) -> Result<WaitLiveState, AegisError> {
        let selector_json = serde_json::to_string(&selector).map_err(AegisError::Serialize)?;
        let script = format!("JSON.stringify(window.__aegis.waitState({selector_json}))");
        let raw = self.bridge.eval_js(&script)?;
        serde_json::from_str(&raw)
            .map_err(|error| AegisError::Bridge(format!("wait state json parse error: {error}")))
    }

    fn refresh_live_state(&mut self, force: bool) -> Result<(), AegisError> {
        if !force
            && self
                .last_live_state_refresh_at_ms
                .is_some_and(|last| now_ms().saturating_sub(last) < LIVE_STATE_REFRESH_INTERVAL_MS)
        {
            return Ok(());
        }

        if !self.host_state.runtime_ready {
            return Ok(());
        }

        let script = r#"JSON.stringify(window.__aegis ? window.__aegis.currentPageState() : {
            url: window.location ? window.location.href : null,
            title: document.title || null,
            ready_state: document.readyState || null,
            media: []
        })"#;
        let raw = self.bridge.eval_js(script)?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|error| AegisError::Bridge(format!("live state json parse error: {error}")))?;
        let live_url = value
            .get("url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| self.current_url.clone());
        let synthetic_shell_active = value
            .get("bootstrap")
            .and_then(|bootstrap| bootstrap.get("synthetic_shell_active"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        self.current_url = if synthetic_shell_active {
            self.host_state
                .current_url
                .clone()
                .or(live_url.clone())
                .or_else(|| self.current_url.clone())
        } else {
            live_url
        };
        self.current_title = value
            .get("title")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        self.document_ready_state = value
            .get("ready_state")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        self.page_bootstrap = value
            .get("bootstrap")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| {
                AegisError::Bridge(format!("page bootstrap json parse error: {error}"))
            })?
            .unwrap_or_default();
        self.media = value
            .get("media")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| AegisError::Bridge(format!("media state json parse error: {error}")))?
            .unwrap_or_default();
        self.last_live_state_refresh_at_ms = Some(now_ms());
        self.mark_successful_bridge_roundtrip();
        Ok(())
    }

    fn refresh_host_state(&mut self) -> Result<(), AegisError> {
        let host_state = self.bridge.snapshot_host_state()?;
        if let Some(url) = host_state.current_url.clone() {
            self.current_url = Some(url);
        }
        self.host_state = host_state;
        self.mark_successful_bridge_roundtrip();
        self.try_recover_runtime_bridge()?;
        Ok(())
    }

    fn try_recover_runtime_bridge(&mut self) -> Result<(), AegisError> {
        if !self.host_state.browser_available
            || self.host_state.browser_closed
            || self.host_state.load_in_progress
            || !self.host_state.page_ready
            || !self.host_state.renderer_ready
            || self.host_state.runtime_ready
            || self.host_state.cancel_requested
        {
            return Ok(());
        }

        self.bridge.ensure_runtime()?;
        self.mark_successful_bridge_roundtrip();
        let _ = self.drain_pending_events();
        self.host_state = self.bridge.snapshot_host_state()?;
        self.mark_successful_bridge_roundtrip();
        if self.host_state.runtime_ready {
            let _ = self.refresh_live_state(true);
        }
        Ok(())
    }

    fn mark_successful_bridge_roundtrip(&mut self) {
        self.last_successful_bridge_roundtrip_at_ms = Some(now_ms());
    }

    fn mark_successful_command(&mut self) {
        let now = now_ms();
        self.last_successful_bridge_roundtrip_at_ms = Some(now);
        self.last_successful_command_at_ms = Some(now);
    }

    fn annotate_command_result(
        &self,
        command: &Command,
        mut result: CommandResult,
        prior_event_sequence: Option<u64>,
        emitted_events: &[SequencedEvent],
    ) -> CommandResult {
        if !result.ok {
            return result;
        }
        let Some(Value::Object(value)) = result.value.take() else {
            return result;
        };
        let enriched = match command {
            Command::Click { .. } | Command::PressKey { .. } => {
                annotate_interaction_value(value, prior_event_sequence, emitted_events)
            }
            _ => Value::Object(value),
        };
        result.value = Some(enriched);
        result
    }

    fn resolve_live_target(
        &mut self,
        target: &CommandTarget,
        action: Option<&str>,
    ) -> Result<LiveResolvedTarget, AegisError> {
        let target_json = serde_json::to_string(target).map_err(AegisError::Serialize)?;
        let action_json = serde_json::to_string(&action).map_err(AegisError::Serialize)?;
        let script = format!(
            "JSON.stringify(window.__aegis.resolveTargetInfo({target_json}, {action_json}))"
        );
        let raw = self.bridge.eval_js(&script)?;
        serde_json::from_str(&raw).map_err(|error| {
            AegisError::Bridge(format!("target resolution json parse error: {error}"))
        })
    }
}

fn annotate_interaction_value(
    mut value: Map<String, Value>,
    prior_event_sequence: Option<u64>,
    emitted_events: &[SequencedEvent],
) -> Value {
    let submit_intent = value
        .get("submit_intent")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let navigation_changed = value
        .get("navigation_changed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let title_changed = value
        .get("title_changed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let submit_event_fired = value
        .get("submit_tracking")
        .and_then(|tracking| tracking.get("submit_event"))
        .and_then(|event| event.get("fired"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let submit_default_prevented = value
        .get("submit_tracking")
        .and_then(|tracking| tracking.get("submit_event"))
        .and_then(|event| event.get("default_prevented"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let invalid_controls = value
        .get("submit_tracking")
        .and_then(|tracking| tracking.get("form_after"))
        .and_then(|form| form.get("invalid_controls"))
        .and_then(Value::as_array)
        .map(|controls| controls.len())
        .unwrap_or(0);
    let auth_signals = value
        .get("auth_diagnostics")
        .and_then(|auth| auth.get("signals"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let signal_kinds = auth_signals
        .iter()
        .filter_map(|signal| signal.get("kind").and_then(Value::as_str))
        .collect::<Vec<_>>();

    let network_events = emitted_events
        .iter()
        .filter_map(|entry| match &entry.event {
            RuntimeEvent::Network {
                url,
                phase,
                status,
                error_text,
                ..
            } => Some(json!({
                "url": url,
                "phase": phase,
                "status": status,
                "error_text": error_text,
            })),
            _ => None,
        })
        .collect::<Vec<_>>();
    let websocket_events = emitted_events
        .iter()
        .filter(|entry| matches!(entry.event, RuntimeEvent::WebSocketFrame { .. }))
        .count();
    let network_requests = emitted_events
        .iter()
        .filter(|entry| {
            matches!(
                entry.event,
                RuntimeEvent::Network {
                    phase: Some(crate::events::stream::NetworkResourcePhase::Request),
                    ..
                }
            )
        })
        .count();
    let network_responses = emitted_events
        .iter()
        .filter(|entry| {
            matches!(
                entry.event,
                RuntimeEvent::Network {
                    phase: Some(crate::events::stream::NetworkResourcePhase::Response)
                        | Some(crate::events::stream::NetworkResourcePhase::Finished),
                    ..
                }
            )
        })
        .count();
    let network_failures = emitted_events
        .iter()
        .filter(|entry| {
            matches!(
                entry.event,
                RuntimeEvent::Network {
                    phase: Some(crate::events::stream::NetworkResourcePhase::Failed),
                    ..
                }
            )
        })
        .count();

    let outcome = if signal_kinds.contains(&"redirect_loop") {
        "redirect_loop"
    } else if signal_kinds.contains(&"upstream_failure") {
        "upstream_failure"
    } else if signal_kinds.contains(&"captcha_blocked") {
        "captcha_blocked"
    } else if signal_kinds.contains(&"credentials_rejected") {
        "credentials_rejected"
    } else if submit_default_prevented {
        "submit_prevented"
    } else if invalid_controls > 0 {
        "validation_blocked"
    } else if navigation_changed || title_changed {
        "navigation_observed"
    } else if network_failures > 0 {
        "network_request_failed"
    } else if network_requests > 0 || network_responses > 0 {
        "request_attempted_same_page"
    } else if submit_intent && !submit_event_fired {
        "submit_not_observed"
    } else if submit_intent {
        "submit_dispatched_without_progress"
    } else {
        "interaction_only"
    };

    value.insert(
        "interaction_diagnostics".into(),
        json!({
            "outcome": outcome,
            "submit_intent": submit_intent,
            "submit_event_fired": submit_event_fired,
            "submit_default_prevented": submit_default_prevented,
            "invalid_control_count": invalid_controls,
            "network_request_count": network_requests,
            "network_response_count": network_responses,
            "network_failure_count": network_failures,
            "network_events": network_events,
            "websocket_frame_count": websocket_events,
            "auth_signal_kinds": signal_kinds,
            "emitted_event_count": emitted_events.len(),
            "prior_event_sequence": prior_event_sequence,
        }),
    );
    Value::Object(value)
}

fn extract_command_event_span(value: &mut Option<Value>) -> Option<CommandEventSpan> {
    let object = value.as_mut()?.as_object_mut()?;
    let aegis = object.remove("_aegis")?;
    let span = aegis.get("event_span")?;
    Some(CommandEventSpan {
        start: span.get("start")?.as_u64()? as usize,
        end: span.get("end")?.as_u64()? as usize,
    })
}

fn command_events_for_span(
    emitted_events: &[SequencedEvent],
    span: Option<CommandEventSpan>,
    command_count: usize,
) -> &[SequencedEvent] {
    let Some(span) = span else {
        return if command_count == 1 {
            emitted_events
        } else {
            &[]
        };
    };
    if span.start > span.end || span.end > emitted_events.len() {
        return if command_count == 1 {
            emitted_events
        } else {
            &[]
        };
    }
    &emitted_events[span.start..span.end]
}

fn command_prior_event_sequence(
    previous_global_sequence: Option<u64>,
    emitted_events: &[SequencedEvent],
    span: Option<CommandEventSpan>,
) -> Option<u64> {
    let Some(span) = span else {
        return previous_global_sequence;
    };
    if span.start == 0 {
        previous_global_sequence
    } else {
        emitted_events
            .get(span.start.saturating_sub(1))
            .map(|event| event.sequence)
            .or(previous_global_sequence)
    }
}

fn commands_need_interaction_diagnostics(commands: &[Command]) -> bool {
    commands.iter().any(|command| {
        matches!(
            command,
            Command::Click { .. } | Command::PressKey { .. } | Command::SetValue { .. }
        )
    })
}

impl WaitEventSummary {
    fn from_events(events: &[SequencedEvent]) -> Self {
        let mut summary = Self::default();
        for entry in events {
            match &entry.event {
                RuntimeEvent::DomMutation { .. } => summary.dom_changed = true,
                RuntimeEvent::Navigation { .. } => summary.navigation_changed = true,
                RuntimeEvent::Network { .. } => summary.network_changed = true,
                RuntimeEvent::Log { message, .. } => {
                    summary.log_changed = true;
                    if message.starts_with("media:") {
                        summary.media_signal_changed = true;
                    }
                }
                _ => {}
            }
        }
        summary
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

fn load_upload_payloads(
    paths: &[PathBuf],
    upload_dir_override: Option<&Path>,
) -> Result<Vec<UploadFilePayload>, CommandResult> {
    paths
        .iter()
        .map(|path| load_upload_payload(path, upload_dir_override))
        .collect()
}

fn load_upload_payload(
    path: &Path,
    upload_dir_override: Option<&Path>,
) -> Result<UploadFilePayload, CommandResult> {
    let upload_dir = upload_dir_override
        .map(Path::to_path_buf)
        .or_else(|| {
            crate::state::AegisStatePaths::detect()
                .ok()
                .map(|paths| paths.uploads_dir())
        })
        .ok_or_else(|| CommandResult::err("failed to resolve upload staging directory"))?;
    let staged = stage_upload_file(path, &upload_dir).map_err(CommandResult::err)?;
    let bytes = fs::read(&staged.staged_path).map_err(|error| {
        CommandResult::err(format!(
            "failed to read staged upload file {}: {error}",
            staged.staged_path.display()
        ))
    })?;
    let metadata = fs::metadata(path).map_err(|error| {
        CommandResult::err(format!(
            "failed to stat upload file {}: {error}",
            path.display()
        ))
    })?;
    let last_modified_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as u64);
    Ok(UploadFilePayload {
        name: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("upload.bin")
            .to_string(),
        mime_type: infer_mime_type(path),
        base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        last_modified_ms,
    })
}

fn infer_mime_type(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let mime = match extension.as_str() {
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    };
    Some(mime.to_string())
}
