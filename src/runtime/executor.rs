use crate::browser::BrowserConfig;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
}

const LIVE_STATE_REFRESH_INTERVAL_MS: u64 = 250;
const DEFAULT_WAIT_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_WAIT_POLL_INTERVAL_MS: u64 = 50;
const MIN_WAIT_POLL_INTERVAL_MS: u64 = 10;

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
