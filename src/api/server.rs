use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::oneshot;
use tokio::sync::watch;
use tokio::time::timeout;

use crate::browser::BrowserConfig;
use crate::commands::command::{Command, CommandTarget};
use crate::commands::matcher::resolve_command_target as resolve_snapshot_target;
use crate::config_store::{
    AegisConfigStore, AegisSecretStore, CredentialInput, CredentialsSettings, StoredCredentialEntry,
};
use crate::display::{
    DashboardBootstrap, LinuxDisplayStack, open_dashboard, set_display_env,
    spawn_linux_display_stack,
};
use crate::dom::node::{DomNode, DomSnapshot};
use crate::events::stream::{EventReadWindow, SequencedEvent};
use crate::host::LoadedAegisClient;
use crate::runtime::executor::{
    ExecutionReport, PageResearchControl, PageResearchData, PageResearchForm, PageResearchHeading,
    PageResearchLink, RuntimeStatus, RuntimeTelemetrySnapshot,
};
use crate::session::cookies::SessionState;
use crate::session::profile::{SessionProfileInfo, SessionProfileStore};
use crate::transport::bridge::{AegisError, BrowserChromeState};

const IDLE_PUMP_INTERVAL: Duration = Duration::from_millis(10);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);
const RECENT_OPERATION_LIMIT: usize = 64;
const DASHBOARD_RESOLUTION: &str = "1440x960x24";

#[derive(Clone)]
pub struct ApiState {
    tx: mpsc::Sender<ApiCommand>,
    host_library: PathBuf,
    browser: BrowserConfig,
    startup: Arc<Mutex<ServeStartupMetrics>>,
    profile: SessionProfileInfo,
    diagnostics: Arc<Mutex<ServeDiagnostics>>,
    chrome_tx: Arc<watch::Sender<BrowserChromeState>>,
    tabs_tx: Arc<watch::Sender<BrowserUiState>>,
    dashboard_bootstrap: Option<DashboardBootstrap>,
    vnc_addr: Option<SocketAddr>,
}

impl ApiState {
    pub fn chrome_rx(&self) -> watch::Receiver<BrowserChromeState> {
        self.chrome_tx.subscribe()
    }

    pub fn chrome_state_snapshot(&self) -> BrowserChromeState {
        self.chrome_tx.borrow().clone()
    }

    pub fn tabs_rx(&self) -> watch::Receiver<BrowserUiState> {
        self.tabs_tx.subscribe()
    }

    pub fn tabs_state_snapshot(&self) -> BrowserUiState {
        self.tabs_tx.borrow().clone()
    }

    #[allow(clippy::result_large_err)]
    pub(crate) fn send_command(
        &self,
        command: ApiCommand,
    ) -> Result<(), mpsc::SendError<ApiCommand>> {
        self.tx.send(command)
    }

    pub fn dashboard_bootstrap(&self) -> Option<DashboardBootstrap> {
        self.dashboard_bootstrap.clone()
    }

    pub fn vnc_addr(&self) -> Option<SocketAddr> {
        self.vnc_addr
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ServeStartupMetrics {
    client_connect_ms: u64,
    api_bind_ms: u64,
    total_ready_ms: u64,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    control_plane_up: bool,
    runtime_state: RuntimeOperationalState,
    command_ready: bool,
    bridge_healthy: bool,
    browser_backend_healthy: bool,
    active_operation: Option<OperationSnapshot>,
    last_failure: Option<FailureSnapshot>,
}

#[derive(Debug, Deserialize)]
pub struct NavigateBody {
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteBody {
    pub commands: Vec<Command>,
}

#[derive(Debug, Deserialize)]
pub struct TraceBody {
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct SearchBody {
    pub query: String,
    #[serde(default)]
    pub engine: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PageFindBody {
    pub text: String,
    #[serde(default)]
    pub exact: bool,
}

#[derive(Debug, Deserialize)]
pub struct PageOpenLinkBody {
    pub text: String,
    #[serde(default)]
    pub exact: bool,
    #[serde(default)]
    pub href_contains: Option<String>,
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PageReadQuery {
    #[serde(default)]
    pub tab_id: Option<u64>,
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub engine: String,
    pub query: String,
    pub url: String,
    pub events: Vec<SequencedEvent>,
}

#[derive(Debug, Serialize)]
pub struct PageTextResponse {
    pub scope: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub text: String,
    pub page_type: String,
    pub useful_text_available: bool,
    pub interactive_elements_available: bool,
    pub blocked_by_overlay: bool,
    pub blocker_signals: Vec<String>,
    pub suggested_next_actions: Vec<String>,
    pub likely_not_found: bool,
    pub not_found_signals: Vec<String>,
    pub suggested_search_query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageMarkdownResponse {
    pub scope: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub markdown: String,
    pub page_type: String,
    pub useful_text_available: bool,
    pub interactive_elements_available: bool,
    pub blocked_by_overlay: bool,
    pub blocker_signals: Vec<String>,
    pub suggested_next_actions: Vec<String>,
    pub likely_not_found: bool,
    pub not_found_signals: Vec<String>,
    pub suggested_search_query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageLinksResponse {
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub links: Vec<PageResearchLink>,
    pub likely_not_found: bool,
    pub suggested_search_query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageHeadingsResponse {
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub headings: Vec<PageResearchHeading>,
    pub likely_not_found: bool,
    pub suggested_search_query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageActionsResponse {
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub page_type: String,
    pub useful_text_available: bool,
    pub interactive_elements_available: bool,
    pub blocked_by_overlay: bool,
    pub blocker_signals: Vec<String>,
    pub primary_links: Vec<PageResearchLink>,
    pub primary_controls: Vec<PageResearchControl>,
    pub suggested_next_actions: Vec<String>,
    pub suggested_search_query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageFormsResponse {
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub page_type: String,
    pub auth_wall_likely: bool,
    pub blocked_by_overlay: bool,
    pub blocker_signals: Vec<String>,
    pub forms: Vec<PageResearchForm>,
    pub suggested_next_actions: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PageFindMatch {
    pub kind: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageFindResponse {
    pub query: String,
    pub exact: bool,
    pub title: Option<String>,
    pub url: Option<String>,
    pub canonical_url: Option<String>,
    pub match_count: usize,
    pub matches: Vec<PageFindMatch>,
    pub likely_not_found: bool,
    pub suggested_search_query: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PageOpenLinkResponse {
    pub text: String,
    pub exact: bool,
    pub href_contains: Option<String>,
    pub candidate_count: usize,
    pub chosen: PageResearchLink,
    pub events: Vec<SequencedEvent>,
}

#[derive(Debug, Deserialize)]
pub struct EventQuery {
    #[serde(default)]
    pub since: u64,
    #[serde(default)]
    pub tab_id: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct TabQuery {
    #[serde(default)]
    pub tab_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct TabCreateBody {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub source_tab_id: Option<u64>,
    #[serde(default = "default_true")]
    pub activate: bool,
    #[serde(default = "default_true")]
    pub inherit_session: bool,
}

#[derive(Debug, Deserialize)]
pub struct TabIdBody {
    pub tab_id: u64,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserTabState {
    pub id: u64,
    pub title: String,
    pub url: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub is_loading: bool,
}

impl BrowserTabState {
    fn blank(id: u64, url: Option<String>) -> Self {
        let url = url.unwrap_or_default();
        Self {
            id,
            title: if url.is_empty() {
                "New Tab".into()
            } else {
                url.clone()
            },
            url,
            can_go_back: false,
            can_go_forward: false,
            is_loading: false,
        }
    }

    fn apply_chrome_state(&mut self, state: &BrowserChromeState) {
        self.title = if state.title.trim().is_empty() {
            "New Tab".into()
        } else {
            state.title.clone()
        };
        self.url = state.url.clone();
        self.can_go_back = state.can_go_back;
        self.can_go_forward = state.can_go_forward;
        self.is_loading = state.is_loading;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrowserUiState {
    pub active_tab_id: u64,
    pub tabs: Vec<BrowserTabState>,
}

impl Default for BrowserUiState {
    fn default() -> Self {
        Self {
            active_tab_id: 1,
            tabs: vec![BrowserTabState::blank(1, None)],
        }
    }
}

struct BrowserTabController {
    next_tab_id: u64,
    active_tab_id: u64,
    tabs: BTreeMap<u64, ManagedTab>,
}

struct ManagedTab {
    state: BrowserTabState,
    client: LoadedAegisClient,
    credential_capture: AutoCredentialCapture,
}

#[derive(Debug, Clone, Serialize)]
pub struct TabOperationResponse {
    pub tab: BrowserTabState,
    pub tabs: BrowserUiState,
}

impl BrowserTabController {
    fn new(initial_client: LoadedAegisClient) -> Self {
        let mut tabs = BTreeMap::new();
        tabs.insert(
            1,
            ManagedTab {
                state: BrowserTabState::blank(1, None),
                client: initial_client,
                credential_capture: AutoCredentialCapture::default(),
            },
        );
        Self {
            next_tab_id: 2,
            active_tab_id: 1,
            tabs,
        }
    }

    fn snapshot(&self) -> BrowserUiState {
        BrowserUiState {
            active_tab_id: self.active_tab_id,
            tabs: self.tabs.values().map(|tab| tab.state.clone()).collect(),
        }
    }

    fn active_tab_id(&self) -> u64 {
        self.active_tab_id
    }

    fn resolve_tab_id(&self, requested: Option<u64>) -> Result<u64, AegisError> {
        let id = requested.unwrap_or(self.active_tab_id);
        if self.tabs.contains_key(&id) {
            Ok(id)
        } else {
            Err(AegisError::Bridge(format!("tab `{id}` does not exist")))
        }
    }

    fn active_tab_state(&self) -> Option<BrowserTabState> {
        self.tabs
            .get(&self.active_tab_id)
            .map(|tab| tab.state.clone())
    }

    fn active_client(&self) -> Result<&LoadedAegisClient, AegisError> {
        Ok(&self.get_tab(self.active_tab_id)?.client)
    }

    fn active_client_mut(&mut self) -> Result<&mut LoadedAegisClient, AegisError> {
        Ok(&mut self.get_tab_mut(self.active_tab_id)?.client)
    }

    fn tab_client(&self, tab_id: u64) -> Result<&LoadedAegisClient, AegisError> {
        Ok(&self.get_tab(tab_id)?.client)
    }

    fn tab_client_mut(&mut self, tab_id: u64) -> Result<&mut LoadedAegisClient, AegisError> {
        Ok(&mut self.get_tab_mut(tab_id)?.client)
    }

    fn tab_state(&self, tab_id: u64) -> Result<BrowserTabState, AegisError> {
        self.tabs
            .get(&tab_id)
            .map(|tab| tab.state.clone())
            .ok_or_else(|| AegisError::Bridge(format!("tab `{tab_id}` does not exist")))
    }

    fn get_tab_mut(&mut self, tab_id: u64) -> Result<&mut ManagedTab, AegisError> {
        self.tabs
            .get_mut(&tab_id)
            .ok_or_else(|| AegisError::Bridge(format!("tab `{tab_id}` does not exist")))
    }

    fn get_tab(&self, tab_id: u64) -> Result<&ManagedTab, AegisError> {
        self.tabs
            .get(&tab_id)
            .ok_or_else(|| AegisError::Bridge(format!("tab `{tab_id}` does not exist")))
    }

    fn apply_chrome_state(
        &mut self,
        tab_id: u64,
        state: &BrowserChromeState,
    ) -> Result<(), AegisError> {
        let tab = self.get_tab_mut(tab_id)?;
        tab.state.apply_chrome_state(state);
        Ok(())
    }

    fn refresh_tab_state(&mut self, tab_id: u64) -> Result<BrowserChromeState, AegisError> {
        let state = self.get_tab_mut(tab_id)?.client.snapshot_chrome_state()?;
        self.apply_chrome_state(tab_id, &state)?;
        Ok(state)
    }

    fn activate_tab(&mut self, tab_id: u64) -> Result<BrowserTabState, AegisError> {
        self.resolve_tab_id(Some(tab_id))?;
        self.active_tab_id = tab_id;
        let _ = self.refresh_tab_state(tab_id);
        self.tab_state(tab_id)
    }

    fn close_tab(&mut self, tab_id: u64) -> Result<Option<BrowserTabState>, AegisError> {
        self.resolve_tab_id(Some(tab_id))?;
        let remaining = self
            .tabs
            .keys()
            .copied()
            .filter(|id| *id != tab_id)
            .collect::<Vec<_>>();
        let was_active = self.active_tab_id == tab_id;
        self.tabs.remove(&tab_id);

        if self.tabs.is_empty() {
            return Ok(None);
        }
        if was_active {
            let replacement_id = remaining
                .last()
                .copied()
                .ok_or_else(|| AegisError::Bridge("failed to resolve replacement tab".into()))?;
            self.active_tab_id = replacement_id;
            let _ = self.refresh_tab_state(replacement_id);
            return Ok(Some(self.tab_state(replacement_id)?));
        }
        Ok(self.active_tab_state())
    }

    fn create_tab(
        &mut self,
        host_library: &Path,
        browser: &BrowserConfig,
        request: TabCreateBody,
    ) -> Result<BrowserTabState, AegisError> {
        let source_tab_id = request
            .source_tab_id
            .or_else(|| (!self.tabs.is_empty()).then_some(self.active_tab_id));
        let inherited_session = if request.inherit_session {
            match source_tab_id {
                Some(source_tab_id) => {
                    Some(self.get_tab_mut(source_tab_id)?.client.snapshot_session()?)
                }
                None => None,
            }
        } else {
            None
        };

        let mut tab_browser = browser.clone();
        tab_browser.start_url = request.url.clone().or_else(|| Some("about:blank".into()));
        let mut client = LoadedAegisClient::connect(host_library, tab_browser)?;
        if let Some(session) = inherited_session {
            client.inject_session(session)?;
        }
        if let Some(url) = request.url.as_ref() {
            let _ = client.navigate(url.clone())?;
        }

        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let mut managed = ManagedTab {
            state: BrowserTabState::blank(id, request.url.clone()),
            client,
            credential_capture: AutoCredentialCapture::default(),
        };
        if let Ok(chrome) = managed.client.snapshot_chrome_state() {
            managed.state.apply_chrome_state(&chrome);
        }
        let state = managed.state.clone();
        self.tabs.insert(id, managed);
        if request.activate {
            self.active_tab_id = id;
        }
        Ok(state)
    }

    fn ensure_not_empty(
        &mut self,
        host_library: &Path,
        browser: &BrowserConfig,
    ) -> Result<BrowserTabState, AegisError> {
        if let Some(state) = self.active_tab_state() {
            return Ok(state);
        }
        let state = self.create_tab(
            host_library,
            browser,
            TabCreateBody {
                url: Some("about:blank".into()),
                source_tab_id: None,
                activate: true,
                inherit_session: false,
            },
        )?;
        Ok(state)
    }

    fn pump_all(&mut self) -> Result<Option<BrowserChromeState>, AegisError> {
        let ids = self.tabs.keys().copied().collect::<Vec<_>>();
        let mut active_chrome = None;
        for id in ids {
            {
                let tab = self.get_tab_mut(id)?;
                tab.client.pump()?;
            }
            if let Ok(state) = self.refresh_tab_state(id)
                && id == self.active_tab_id
            {
                active_chrome = Some(state);
            }
        }
        Ok(active_chrome)
    }
}

pub(crate) enum ApiCommand {
    InjectSession(
        Option<u64>,
        SessionState,
        oneshot::Sender<Result<(), AegisError>>,
    ),
    SnapshotSession(
        Option<u64>,
        oneshot::Sender<Result<SessionState, AegisError>>,
    ),
    SnapshotTelemetry(
        Option<u64>,
        oneshot::Sender<Result<TelemetryResponse, AegisError>>,
    ),
    SaveSessionProfile(
        Option<u64>,
        oneshot::Sender<Result<SessionProfileInfo, AegisError>>,
    ),
    LoadSessionProfile(
        Option<u64>,
        oneshot::Sender<Result<SessionProfileInfo, AegisError>>,
    ),
    Navigate(
        Option<u64>,
        String,
        oneshot::Sender<Result<Vec<SequencedEvent>, AegisError>>,
    ),
    Execute(
        Option<u64>,
        Vec<Command>,
        oneshot::Sender<Result<ExecutionReport, AegisError>>,
    ),
    PageResearch(
        Option<u64>,
        oneshot::Sender<Result<PageResearchData, AegisError>>,
    ),
    SnapshotDom(
        Option<u64>,
        oneshot::Sender<Result<DomSnapshot, AegisError>>,
    ),
    Events(
        Option<u64>,
        u64,
        oneshot::Sender<Result<EventReadWindow, AegisError>>,
    ),
    EnableTrace(
        Option<u64>,
        PathBuf,
        oneshot::Sender<Result<(), AegisError>>,
    ),
    GoBack(Option<u64>),
    GoForward(Option<u64>),
    Reload(Option<u64>),
    StopLoad(Option<u64>),
    ChromeNavigate(Option<u64>, String),
    ListTabs(oneshot::Sender<Result<BrowserUiState, AegisError>>),
    CreateTab(
        TabCreateBody,
        oneshot::Sender<Result<TabOperationResponse, AegisError>>,
    ),
    ActivateTab(
        u64,
        oneshot::Sender<Result<TabOperationResponse, AegisError>>,
    ),
    CloseTab(u64, oneshot::Sender<Result<BrowserUiState, AegisError>>),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RuntimeOperationalState {
    Starting,
    Ready,
    Busy,
    Degraded,
    Wedged,
}

#[derive(Debug, Clone, Serialize)]
struct OperationSnapshot {
    id: u64,
    name: String,
    stage: String,
    started_at_ms: u64,
    elapsed_ms: u64,
    timed_out: bool,
}

#[derive(Debug, Clone, Serialize)]
struct FailureSnapshot {
    operation: String,
    stage: String,
    message: String,
    elapsed_ms: u64,
    timed_out: bool,
    restart_recommended: bool,
    first_seen_at_ms: u64,
    last_seen_at_ms: u64,
}

#[derive(Debug, Clone)]
struct ActiveOperation {
    id: u64,
    name: String,
    stage: String,
    started_at_ms: u64,
    started_at: Instant,
    timed_out: bool,
}

#[derive(Debug, Clone)]
struct ServeDiagnostics {
    runtime: RuntimeStatus,
    active_operation: Option<ActiveOperation>,
    last_failure: Option<FailureSnapshot>,
    total_operations: u64,
    successful_operations: u64,
    timed_out_operations: u64,
    next_operation_id: u64,
    recent_operations: VecDeque<CompletedOperationSnapshot>,
    operation_aggregates: BTreeMap<String, OperationAggregateState>,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeDiagnosticsResponse {
    state: RuntimeOperationalState,
    control_plane_up: bool,
    command_ready: bool,
    bridge_healthy: bool,
    browser_backend_healthy: bool,
    dom_snapshot_available: bool,
    active_operation: Option<OperationSnapshot>,
    last_failure: Option<FailureSnapshot>,
    total_operations: u64,
    successful_operations: u64,
    timed_out_operations: u64,
    recent_operations: Vec<CompletedOperationSnapshot>,
    operation_aggregates: Vec<OperationAggregateTelemetry>,
    runtime: RuntimeStatus,
}

#[derive(Debug, Clone, Serialize)]
struct CompletedOperationSnapshot {
    id: u64,
    name: String,
    stage: String,
    status: String,
    started_at_ms: u64,
    finished_at_ms: u64,
    elapsed_ms: u64,
    timed_out: bool,
    error_message: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct OperationLatencyHistogram {
    lt_50_ms: u64,
    lt_100_ms: u64,
    lt_250_ms: u64,
    lt_500_ms: u64,
    lt_1000_ms: u64,
    gte_1000_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
struct OperationAggregateTelemetry {
    name: String,
    total_count: u64,
    success_count: u64,
    failure_count: u64,
    timeout_count: u64,
    avg_elapsed_ms: u64,
    min_elapsed_ms: u64,
    max_elapsed_ms: u64,
    last_elapsed_ms: u64,
    histogram: OperationLatencyHistogram,
}

#[derive(Debug, Clone, Default, Serialize)]
struct OperationAggregateState {
    total_count: u64,
    success_count: u64,
    failure_count: u64,
    timeout_count: u64,
    total_elapsed_ms: u64,
    min_elapsed_ms: u64,
    max_elapsed_ms: u64,
    last_elapsed_ms: u64,
    histogram: OperationLatencyHistogram,
}

#[derive(Debug, Clone, Serialize)]
struct SessionCookieTelemetry {
    name: String,
    domain: String,
    path: Option<String>,
    expires_unix: Option<u64>,
    secure: bool,
    http_only: bool,
    value_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
struct SessionStorageEntryTelemetry {
    key: String,
    value_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
struct NetworkOverrideTelemetry {
    header: String,
    value_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
struct SessionTelemetry {
    profile: SessionProfileInfo,
    cookie_count: usize,
    cookies: Vec<SessionCookieTelemetry>,
    local_storage_count: usize,
    local_storage: Vec<SessionStorageEntryTelemetry>,
    session_storage_count: usize,
    session_storage: Vec<SessionStorageEntryTelemetry>,
    network_override_count: usize,
    network_overrides: Vec<NetworkOverrideTelemetry>,
}

#[derive(Debug, Clone, Serialize)]
struct CredentialTelemetryEntry {
    origin: String,
    username: String,
    username_field: Option<String>,
    password_field: Option<String>,
    form_label: Option<String>,
    created_at_ms: u64,
    updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
struct CredentialsTelemetry {
    settings: CredentialsSettings,
    stored_credentials_count: usize,
    entries: Vec<CredentialTelemetryEntry>,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeSettingsTelemetry {
    default_profile: Option<String>,
    headless_persistent: Option<bool>,
    headful_persistent: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
struct DashboardTelemetry {
    headful_dashboard: bool,
    bootstrap: Option<DashboardBootstrap>,
    vnc_addr: Option<String>,
    resolution: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TelemetryResponse {
    tab_id: u64,
    tabs: BrowserUiState,
    host_library: PathBuf,
    browser: BrowserConfig,
    startup: ServeStartupMetrics,
    diagnostics: RuntimeDiagnosticsResponse,
    chrome: BrowserChromeState,
    runtime: RuntimeTelemetrySnapshot,
    session: SessionTelemetry,
    credentials: CredentialsTelemetry,
    settings: RuntimeSettingsTelemetry,
    dashboard: DashboardTelemetry,
}

#[derive(Debug, Deserialize)]
struct NativeOperationError {
    kind: String,
    operation: String,
    stage: String,
    message: String,
    elapsed_ms: u64,
    timed_out: bool,
    restart_recommended: bool,
}

#[derive(Debug, Clone, Default)]
struct AutoCredentialCapture {
    username: Option<CapturedCredentialField>,
    password: Option<CapturedCredentialField>,
    origin: Option<String>,
}

#[derive(Debug, Clone)]
struct CapturedCredentialField {
    value: String,
    field_name: Option<String>,
    label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialFieldKind {
    Username,
    Password,
}

pub async fn serve(
    addr: SocketAddr,
    host_library: PathBuf,
    browser_config: BrowserConfig,
    profile_name: String,
    open_dashboard_browser: bool,
) -> Result<(), AegisError> {
    let serve_started = std::time::Instant::now();
    let headful_dashboard = requires_linux_dashboard(&browser_config);
    let _web_ui_dist = resolve_web_ui_dist(headful_dashboard)?;
    let display_stack = if headful_dashboard {
        let stack = spawn_linux_display_stack()?;
        set_display_env(stack.display());
        Some(stack)
    } else {
        None
    };
    let client_connect_started = std::time::Instant::now();
    let mut initial_client =
        LoadedAegisClient::connect(host_library.clone(), browser_config.clone())?;
    let profile_store = SessionProfileStore::new(profile_name).map_err(AegisError::Bridge)?;
    let credential_settings = AegisConfigStore::detect()
        .and_then(|store| store.load_credentials_settings())
        .map_err(AegisError::Bridge)?;
    let credential_store = AegisSecretStore::detect().map_err(AegisError::Bridge)?;
    if let Some(session) = profile_store.load().map_err(AegisError::Bridge)? {
        initial_client.inject_session(session)?;
    }
    let client_connect_ms = client_connect_started.elapsed().as_millis() as u64;
    let api_bind_started = std::time::Instant::now();
    let (tx, rx) = mpsc::channel::<ApiCommand>();
    let startup = Arc::new(Mutex::new(ServeStartupMetrics {
        client_connect_ms,
        api_bind_ms: 0,
        total_ready_ms: 0,
    }));
    let diagnostics = Arc::new(Mutex::new(ServeDiagnostics::new(
        initial_client.runtime_status(),
    )));
    let (chrome_tx, _chrome_rx) = watch::channel(BrowserChromeState::default());
    let chrome_tx = Arc::new(chrome_tx);
    let mut tab_controller = BrowserTabController::new(initial_client);
    let active_tab_id = tab_controller.active_tab_id();
    if let Ok(chrome) = tab_controller.refresh_tab_state(active_tab_id) {
        publish_chrome_state(&chrome_tx, chrome);
    }
    let initial_tabs = tab_controller.snapshot();
    let (tabs_tx, _tabs_rx) = watch::channel(initial_tabs);
    let tabs_tx = Arc::new(tabs_tx);
    let (startup_tx, startup_rx) = mpsc::channel::<Result<(), String>>();
    let state = ApiState {
        tx,
        host_library,
        browser: browser_config.clone(),
        startup: startup.clone(),
        profile: profile_store.info(),
        diagnostics: diagnostics.clone(),
        chrome_tx: chrome_tx.clone(),
        tabs_tx: tabs_tx.clone(),
        dashboard_bootstrap: display_stack.as_ref().map(LinuxDisplayStack::bootstrap),
        vnc_addr: display_stack.as_ref().map(LinuxDisplayStack::vnc_addr),
    };
    let router_state = state.clone();
    let startup_host_library = state.host_library.clone();

    thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = startup_tx.send(Err(error.to_string()));
                return;
            }
        };

        runtime.block_on(async move {
            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    let _ = startup_tx.send(Ok(()));
                    listener
                }
                Err(error) => {
                    let _ = startup_tx.send(Err(error.to_string()));
                    return;
                }
            };

            let app = router(router_state);
            let _ = axum::serve(listener, app).await;
        });
    });

    match startup_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(AegisError::Bridge(error)),
        Err(error) => return Err(AegisError::Bridge(error.to_string())),
    }

    let startup_metrics = ServeStartupMetrics {
        client_connect_ms,
        api_bind_ms: api_bind_started.elapsed().as_millis() as u64,
        total_ready_ms: serve_started.elapsed().as_millis() as u64,
    };
    if let Ok(mut shared) = startup.lock() {
        *shared = startup_metrics;
    }

    eprintln!(
        "Aegis serve ready on http://{} ({:?}, host: {})",
        addr,
        browser_config.mode,
        startup_host_library.display()
    );

    if open_dashboard_browser && headful_dashboard {
        open_dashboard(&dashboard_url(addr))?;
    }

    let mut tabs = tab_controller;
    loop {
        match rx.recv_timeout(IDLE_PUMP_INTERVAL) {
            Ok(command) => match command {
                ApiCommand::InjectSession(requested_tab_id, session, reply) => {
                    record_operation_started(&diagnostics, "inject_session", "injecting session");
                    let tab_id = tabs.resolve_tab_id(requested_tab_id);
                    let result = match tab_id {
                        Ok(tab_id) => {
                            let result = tabs
                                .tab_client_mut(tab_id)
                                .and_then(|client| client.inject_session(session.clone()))
                                .and_then(|_| {
                                    profile_store
                                        .save(&session)
                                        .map(|_| ())
                                        .map_err(AegisError::Bridge)
                                });
                            refresh_selected_tab_state(&mut tabs, tab_id, &chrome_tx, &tabs_tx);
                            if let Ok(client) = tabs.tab_client(tab_id) {
                                record_operation_finished(
                                    &diagnostics,
                                    "inject_session",
                                    client,
                                    &result,
                                );
                            }
                            result
                        }
                        Err(error) => {
                            record_operation_failure(
                                &diagnostics,
                                "inject_session",
                                failure_from_error(
                                    "inject_session",
                                    "resolving target tab",
                                    &error,
                                ),
                                tabs.active_client()
                                    .ok()
                                    .map(|client| client.runtime_status()),
                            );
                            Err(error)
                        }
                    };
                    let _ = reply.send(result);
                }
                ApiCommand::SnapshotSession(requested_tab_id, reply) => {
                    record_operation_started(
                        &diagnostics,
                        "snapshot_session",
                        "capturing session state",
                    );
                    let tab_id = tabs.resolve_tab_id(requested_tab_id);
                    let result = match tab_id {
                        Ok(tab_id) => {
                            let result = tabs
                                .tab_client_mut(tab_id)
                                .and_then(LoadedAegisClient::snapshot_session);
                            if let Ok(client) = tabs.tab_client(tab_id) {
                                record_operation_finished(
                                    &diagnostics,
                                    "snapshot_session",
                                    client,
                                    &result,
                                );
                            }
                            result
                        }
                        Err(error) => {
                            record_operation_failure(
                                &diagnostics,
                                "snapshot_session",
                                failure_from_error(
                                    "snapshot_session",
                                    "resolving target tab",
                                    &error,
                                ),
                                tabs.active_client()
                                    .ok()
                                    .map(|client| client.runtime_status()),
                            );
                            Err(error)
                        }
                    };
                    let _ = reply.send(result);
                }
                ApiCommand::SnapshotTelemetry(requested_tab_id, reply) => {
                    record_operation_started(
                        &diagnostics,
                        "snapshot_telemetry",
                        "capturing production telemetry snapshot",
                    );
                    let tab_id = tabs.resolve_tab_id(requested_tab_id);
                    let result = match tab_id {
                        Ok(tab_id) => {
                            let tabs_snapshot = tabs.snapshot();
                            let result = tabs.tab_client_mut(tab_id).and_then(|client| {
                                snapshot_telemetry_response(
                                    &state,
                                    &startup,
                                    &diagnostics,
                                    &profile_store,
                                    &credential_store,
                                    &tabs_snapshot,
                                    tab_id,
                                    client,
                                )
                            });
                            if let Ok(client) = tabs.tab_client(tab_id) {
                                record_operation_finished(
                                    &diagnostics,
                                    "snapshot_telemetry",
                                    client,
                                    &result,
                                );
                            }
                            result
                        }
                        Err(error) => {
                            record_operation_failure(
                                &diagnostics,
                                "snapshot_telemetry",
                                failure_from_error(
                                    "snapshot_telemetry",
                                    "resolving target tab",
                                    &error,
                                ),
                                tabs.active_client()
                                    .ok()
                                    .map(|client| client.runtime_status()),
                            );
                            Err(error)
                        }
                    };
                    let _ = reply.send(result);
                }
                ApiCommand::SaveSessionProfile(requested_tab_id, reply) => {
                    record_operation_started(
                        &diagnostics,
                        "save_session_profile",
                        "persisting session profile",
                    );
                    let tab_id = tabs.resolve_tab_id(requested_tab_id);
                    let result = match tab_id {
                        Ok(tab_id) => {
                            let result = tabs
                                .tab_client_mut(tab_id)
                                .and_then(LoadedAegisClient::snapshot_session)
                                .and_then(|session| {
                                    profile_store
                                        .save(&session)
                                        .map(|_| profile_store.info())
                                        .map_err(AegisError::Bridge)
                                });
                            if let Ok(client) = tabs.tab_client(tab_id) {
                                record_operation_finished(
                                    &diagnostics,
                                    "save_session_profile",
                                    client,
                                    &result,
                                );
                            }
                            result
                        }
                        Err(error) => {
                            record_operation_failure(
                                &diagnostics,
                                "save_session_profile",
                                failure_from_error(
                                    "save_session_profile",
                                    "resolving target tab",
                                    &error,
                                ),
                                tabs.active_client()
                                    .ok()
                                    .map(|client| client.runtime_status()),
                            );
                            Err(error)
                        }
                    };
                    let _ = reply.send(result);
                }
                ApiCommand::LoadSessionProfile(requested_tab_id, reply) => {
                    record_operation_started(
                        &diagnostics,
                        "load_session_profile",
                        "loading session profile",
                    );
                    let tab_id = tabs.resolve_tab_id(requested_tab_id);
                    let result = match tab_id {
                        Ok(tab_id) => {
                            let result = profile_store.load().map_err(AegisError::Bridge).and_then(
                                |maybe_session| match maybe_session {
                                    Some(session) => tabs
                                        .tab_client_mut(tab_id)
                                        .and_then(|client| client.inject_session(session))
                                        .map(|_| profile_store.info()),
                                    None => Ok(profile_store.info()),
                                },
                            );
                            refresh_selected_tab_state(&mut tabs, tab_id, &chrome_tx, &tabs_tx);
                            if let Ok(client) = tabs.tab_client(tab_id) {
                                record_operation_finished(
                                    &diagnostics,
                                    "load_session_profile",
                                    client,
                                    &result,
                                );
                            }
                            result
                        }
                        Err(error) => {
                            record_operation_failure(
                                &diagnostics,
                                "load_session_profile",
                                failure_from_error(
                                    "load_session_profile",
                                    "resolving target tab",
                                    &error,
                                ),
                                tabs.active_client()
                                    .ok()
                                    .map(|client| client.runtime_status()),
                            );
                            Err(error)
                        }
                    };
                    let _ = reply.send(result);
                }
                ApiCommand::Navigate(requested_tab_id, url, reply) => {
                    record_operation_started(
                        &diagnostics,
                        "navigate",
                        &format!("navigating to {url}"),
                    );
                    let tab_id = tabs.resolve_tab_id(requested_tab_id);
                    let result = match tab_id {
                        Ok(tab_id) => {
                            let result = (|| -> Result<Vec<SequencedEvent>, AegisError> {
                                let tab = tabs.get_tab_mut(tab_id)?;
                                tab.credential_capture.reset_on_explicit_navigation(&url);
                                tab.client.navigate(url.clone())
                            })();
                            refresh_selected_tab_state(&mut tabs, tab_id, &chrome_tx, &tabs_tx);
                            if let Ok(client) = tabs.tab_client(tab_id) {
                                record_operation_finished(
                                    &diagnostics,
                                    "navigate",
                                    client,
                                    &result,
                                );
                            }
                            result
                        }
                        Err(error) => {
                            record_operation_failure(
                                &diagnostics,
                                "navigate",
                                failure_from_error("navigate", "resolving target tab", &error),
                                tabs.active_client()
                                    .ok()
                                    .map(|client| client.runtime_status()),
                            );
                            Err(error)
                        }
                    };
                    let _ = reply.send(result);
                }
                ApiCommand::Execute(requested_tab_id, commands, reply) => {
                    record_operation_started(
                        &diagnostics,
                        "execute",
                        "executing browser command batch",
                    );
                    let tab_id = tabs.resolve_tab_id(requested_tab_id);
                    let result = match tab_id {
                        Ok(tab_id) => {
                            let result = (|| -> Result<ExecutionReport, AegisError> {
                                let tab = tabs.get_tab_mut(tab_id)?;
                                let maybe_snapshot = if credential_settings.auto_store
                                    && commands.iter().any(|command| {
                                        matches!(
                                            command,
                                            Command::SetValue { .. } | Command::Click { .. }
                                        )
                                    }) {
                                    Some(tab.client.snapshot_dom()?)
                                } else {
                                    None
                                };
                                if let Some(snapshot) = maybe_snapshot.as_ref() {
                                    tab.credential_capture.capture_fields(
                                        snapshot,
                                        tab.client.runtime().current_url(),
                                        &commands,
                                    );
                                }
                                let should_persist = credential_settings.auto_store
                                    && maybe_snapshot.as_ref().is_some_and(|snapshot| {
                                        tab.credential_capture.should_persist(snapshot, &commands)
                                    });
                                let persist_origin = if should_persist {
                                    tab.client.runtime().current_url().map(origin_key)
                                } else {
                                    None
                                };
                                let report = tab.client.execute(&commands)?;
                                if let Some(origin) = persist_origin {
                                    tab.credential_capture.persist(
                                        &credential_store,
                                        &profile_store.info().profile,
                                        &origin,
                                    )?;
                                }
                                Ok(report)
                            })();
                            refresh_selected_tab_state(&mut tabs, tab_id, &chrome_tx, &tabs_tx);
                            if let Ok(client) = tabs.tab_client(tab_id) {
                                record_operation_finished(&diagnostics, "execute", client, &result);
                            }
                            result
                        }
                        Err(error) => {
                            record_operation_failure(
                                &diagnostics,
                                "execute",
                                failure_from_error("execute", "resolving target tab", &error),
                                tabs.active_client()
                                    .ok()
                                    .map(|client| client.runtime_status()),
                            );
                            Err(error)
                        }
                    };
                    let _ = reply.send(result);
                }
                ApiCommand::PageResearch(requested_tab_id, reply) => {
                    record_operation_started(
                        &diagnostics,
                        "page_research",
                        "capturing structured page research snapshot",
                    );
                    let tab_id = tabs.resolve_tab_id(requested_tab_id);
                    let result = match tab_id {
                        Ok(tab_id) => {
                            let result = tabs
                                .tab_client_mut(tab_id)
                                .and_then(LoadedAegisClient::page_research_data);
                            if let Ok(client) = tabs.tab_client(tab_id) {
                                record_operation_finished(
                                    &diagnostics,
                                    "page_research",
                                    client,
                                    &result,
                                );
                            }
                            result
                        }
                        Err(error) => {
                            record_operation_failure(
                                &diagnostics,
                                "page_research",
                                failure_from_error("page_research", "resolving target tab", &error),
                                tabs.active_client()
                                    .ok()
                                    .map(|client| client.runtime_status()),
                            );
                            Err(error)
                        }
                    };
                    let _ = reply.send(result);
                }
                ApiCommand::SnapshotDom(requested_tab_id, reply) => {
                    record_operation_started(
                        &diagnostics,
                        "snapshot_dom",
                        "capturing DOM snapshot",
                    );
                    let tab_id = tabs.resolve_tab_id(requested_tab_id);
                    let result = match tab_id {
                        Ok(tab_id) => {
                            let result = tabs
                                .tab_client_mut(tab_id)
                                .and_then(LoadedAegisClient::snapshot_dom);
                            if let Ok(client) = tabs.tab_client(tab_id) {
                                record_operation_finished(
                                    &diagnostics,
                                    "snapshot_dom",
                                    client,
                                    &result,
                                );
                            }
                            result
                        }
                        Err(error) => {
                            record_operation_failure(
                                &diagnostics,
                                "snapshot_dom",
                                failure_from_error("snapshot_dom", "resolving target tab", &error),
                                tabs.active_client()
                                    .ok()
                                    .map(|client| client.runtime_status()),
                            );
                            Err(error)
                        }
                    };
                    let _ = reply.send(result);
                }
                ApiCommand::Events(requested_tab_id, since, reply) => {
                    record_operation_started(&diagnostics, "events", "draining runtime events");
                    let tab_id = tabs.resolve_tab_id(requested_tab_id);
                    let result = match tab_id {
                        Ok(tab_id) => {
                            let result = tabs
                                .tab_client_mut(tab_id)
                                .and_then(|client| client.events_since(since));
                            if let Ok(client) = tabs.tab_client(tab_id) {
                                record_operation_finished(&diagnostics, "events", client, &result);
                            }
                            result
                        }
                        Err(error) => {
                            record_operation_failure(
                                &diagnostics,
                                "events",
                                failure_from_error("events", "resolving target tab", &error),
                                tabs.active_client()
                                    .ok()
                                    .map(|client| client.runtime_status()),
                            );
                            Err(error)
                        }
                    };
                    let _ = reply.send(result);
                }
                ApiCommand::EnableTrace(requested_tab_id, path, reply) => {
                    record_operation_started(
                        &diagnostics,
                        "enable_trace",
                        "enabling trace recording",
                    );
                    let tab_id = tabs.resolve_tab_id(requested_tab_id);
                    let result = match tab_id {
                        Ok(tab_id) => {
                            if let Ok(client) = tabs.tab_client_mut(tab_id) {
                                client.enable_trace_recording(path);
                            }
                            let result = Ok(());
                            if let Ok(client) = tabs.tab_client(tab_id) {
                                record_operation_finished(
                                    &diagnostics,
                                    "enable_trace",
                                    client,
                                    &result,
                                );
                            }
                            result
                        }
                        Err(error) => {
                            record_operation_failure(
                                &diagnostics,
                                "enable_trace",
                                failure_from_error("enable_trace", "resolving target tab", &error),
                                tabs.active_client()
                                    .ok()
                                    .map(|client| client.runtime_status()),
                            );
                            Err(error)
                        }
                    };
                    let _ = reply.send(result);
                }
                ApiCommand::GoBack(requested_tab_id) => {
                    if let Ok(tab_id) = tabs.resolve_tab_id(requested_tab_id) {
                        let _ = tabs
                            .tab_client_mut(tab_id)
                            .and_then(LoadedAegisClient::go_back);
                        refresh_selected_tab_state(&mut tabs, tab_id, &chrome_tx, &tabs_tx);
                    }
                }
                ApiCommand::GoForward(requested_tab_id) => {
                    if let Ok(tab_id) = tabs.resolve_tab_id(requested_tab_id) {
                        let _ = tabs
                            .tab_client_mut(tab_id)
                            .and_then(LoadedAegisClient::go_forward);
                        refresh_selected_tab_state(&mut tabs, tab_id, &chrome_tx, &tabs_tx);
                    }
                }
                ApiCommand::Reload(requested_tab_id) => {
                    if let Ok(tab_id) = tabs.resolve_tab_id(requested_tab_id) {
                        let _ = tabs
                            .tab_client_mut(tab_id)
                            .and_then(LoadedAegisClient::reload_page);
                        refresh_selected_tab_state(&mut tabs, tab_id, &chrome_tx, &tabs_tx);
                    }
                }
                ApiCommand::StopLoad(requested_tab_id) => {
                    if let Ok(tab_id) = tabs.resolve_tab_id(requested_tab_id) {
                        let _ = tabs
                            .tab_client_mut(tab_id)
                            .and_then(LoadedAegisClient::stop_load);
                        refresh_selected_tab_state(&mut tabs, tab_id, &chrome_tx, &tabs_tx);
                    }
                }
                ApiCommand::ChromeNavigate(requested_tab_id, url) => {
                    if let Ok(tab_id) = tabs.resolve_tab_id(requested_tab_id) {
                        let _ = tabs
                            .tab_client_mut(tab_id)
                            .and_then(|client| client.navigate(url));
                        refresh_selected_tab_state(&mut tabs, tab_id, &chrome_tx, &tabs_tx);
                    }
                }
                ApiCommand::ListTabs(reply) => {
                    let _ = reply.send(Ok(tabs.snapshot()));
                }
                ApiCommand::CreateTab(request, reply) => {
                    let result = tabs
                        .create_tab(&state.host_library, &state.browser, request)
                        .map(|tab| TabOperationResponse {
                            tab,
                            tabs: tabs.snapshot(),
                        });
                    publish_runtime_tab_state(&mut tabs, &chrome_tx, &tabs_tx);
                    let _ = reply.send(result);
                }
                ApiCommand::ActivateTab(tab_id, reply) => {
                    let result = tabs.activate_tab(tab_id).map(|tab| TabOperationResponse {
                        tab,
                        tabs: tabs.snapshot(),
                    });
                    publish_runtime_tab_state(&mut tabs, &chrome_tx, &tabs_tx);
                    let _ = reply.send(result);
                }
                ApiCommand::CloseTab(tab_id, reply) => {
                    let result = tabs.close_tab(tab_id).and_then(|_| {
                        let _ = tabs.ensure_not_empty(&state.host_library, &state.browser)?;
                        Ok(tabs.snapshot())
                    });
                    publish_runtime_tab_state(&mut tabs, &chrome_tx, &tabs_tx);
                    let _ = reply.send(result);
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => match tabs.pump_all() {
                Ok(active_state) => {
                    if let Ok(client) = tabs.active_client() {
                        record_heartbeat(&diagnostics, client);
                    }
                    if let Some(state) = active_state {
                        publish_chrome_state(&chrome_tx, state);
                    }
                    publish_tabs_state(&tabs_tx, tabs.snapshot());
                }
                Err(error) => {
                    record_operation_failure(
                        &diagnostics,
                        "pump",
                        failure_from_error("pump", "pumping browser event loop", &error),
                        tabs.active_client()
                            .ok()
                            .map(|client| client.runtime_status()),
                    );
                    return Err(error);
                }
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if let Ok(session) = tabs
        .active_client_mut()
        .and_then(LoadedAegisClient::snapshot_session)
    {
        let _ = profile_store.save(&session);
    }

    Ok(())
}

impl AutoCredentialCapture {
    fn capture_fields(
        &mut self,
        snapshot: &DomSnapshot,
        current_url: Option<&str>,
        commands: &[Command],
    ) {
        let current_origin = current_url.map(origin_key);
        if let (Some(existing), Some(current)) = (self.origin.as_ref(), current_origin.as_ref())
            && existing != current
        {
            self.clear();
        }
        if self.origin.is_none() {
            self.origin = current_origin;
        }

        for command in commands {
            let Command::SetValue { target, value } = command else {
                continue;
            };
            let Some(node) = resolve_command_target(snapshot, target) else {
                continue;
            };
            let Some(kind) = classify_credential_field(node) else {
                continue;
            };
            let field = CapturedCredentialField {
                value: value.clone(),
                field_name: node.attrs.get("name").cloned(),
                label: node
                    .semantic
                    .as_ref()
                    .and_then(|semantic| semantic.label.clone().or_else(|| semantic.name.clone())),
            };
            match kind {
                CredentialFieldKind::Username => self.username = Some(field),
                CredentialFieldKind::Password => self.password = Some(field),
            }
        }
    }

    fn should_persist(&self, snapshot: &DomSnapshot, commands: &[Command]) -> bool {
        self.username.is_some()
            && self.password.is_some()
            && commands.iter().any(|command| {
                let Command::Click { target } = command else {
                    return false;
                };
                resolve_command_target(snapshot, target).is_some_and(is_submit_like_node)
            })
    }

    fn persist(
        &mut self,
        store: &AegisSecretStore,
        profile: &str,
        fallback_origin: &str,
    ) -> Result<(), AegisError> {
        let Some(username) = self.username.as_ref() else {
            return Ok(());
        };
        let Some(password) = self.password.as_ref() else {
            return Ok(());
        };
        store
            .upsert_profile_credential(
                profile,
                CredentialInput {
                    origin: self
                        .origin
                        .clone()
                        .unwrap_or_else(|| fallback_origin.to_string()),
                    username: username.value.clone(),
                    password: password.value.clone(),
                    username_field: username.field_name.clone(),
                    password_field: password.field_name.clone(),
                    form_label: password.label.clone().or_else(|| username.label.clone()),
                },
            )
            .map_err(AegisError::Bridge)?;
        self.clear();
        Ok(())
    }

    fn reset_on_explicit_navigation(&mut self, url: &str) {
        let target_origin = origin_key(url);
        if self
            .origin
            .as_ref()
            .is_some_and(|origin| origin != &target_origin)
        {
            self.clear();
        }
    }

    fn clear(&mut self) {
        self.username = None;
        self.password = None;
        self.origin = None;
    }
}

fn resolve_command_target<'a>(
    snapshot: &'a DomSnapshot,
    target: &CommandTarget,
) -> Option<&'a DomNode> {
    resolve_snapshot_target(snapshot, target, None)
}

fn classify_credential_field(node: &DomNode) -> Option<CredentialFieldKind> {
    let control_type = node
        .semantic
        .as_ref()
        .and_then(|semantic| semantic.control_type.as_deref())
        .or_else(|| node.attrs.get("type").map(String::as_str))
        .unwrap_or("text");
    if includes_normalized(Some(control_type), "password") {
        return Some(CredentialFieldKind::Password);
    }
    let hint = credential_hint_text(node);
    if matches!(
        control_type,
        "email" | "text" | "searchbox" | "search" | "textbox"
    ) && (hint.contains("user")
        || hint.contains("email")
        || hint.contains("login")
        || hint.contains("account")
        || hint.contains("identifier")
        || hint.contains("member"))
    {
        return Some(CredentialFieldKind::Username);
    }
    None
}

fn is_submit_like_node(node: &DomNode) -> bool {
    let semantic = node.semantic.as_ref();
    if semantic.is_some_and(|semantic| semantic.actions.iter().any(|action| action == "submit")) {
        return true;
    }
    if semantic
        .as_ref()
        .and_then(|semantic| semantic.control_type.as_deref())
        .is_some_and(|control| matches!(control, "submit" | "button"))
    {
        return true;
    }
    let text = credential_hint_text(node);
    text.contains("sign in")
        || text.contains("log in")
        || text.contains("login")
        || text.contains("continue")
        || text.contains("submit")
}

fn credential_hint_text(node: &DomNode) -> String {
    let mut parts = Vec::new();
    if let Some(text) = node.text.as_ref() {
        parts.push(text.as_str());
    }
    for key in [
        "name",
        "type",
        "placeholder",
        "title",
        "autocomplete",
        "aria-label",
        "value",
    ] {
        if let Some(value) = node.attrs.get(key) {
            parts.push(value.as_str());
        }
    }
    if let Some(semantic) = node.semantic.as_ref() {
        if let Some(name) = semantic.name.as_ref() {
            parts.push(name.as_str());
        }
        if let Some(label) = semantic.label.as_ref() {
            parts.push(label.as_str());
        }
        if let Some(control_type) = semantic.control_type.as_ref() {
            parts.push(control_type.as_str());
        }
    }
    normalize_text(&parts.join(" "))
}

fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn includes_normalized(haystack: Option<&str>, needle: &str) -> bool {
    haystack
        .map(normalize_text)
        .is_some_and(|haystack| haystack.contains(&normalize_text(needle)))
}

fn percent_encode(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        let ch = byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            output.push(ch);
        } else if ch == ' ' {
            output.push('+');
        } else {
            output.push('%');
            output.push_str(&format!("{byte:02X}"));
        }
    }
    output
}

fn search_engine_name(engine: Option<&str>) -> &'static str {
    match engine.map(normalize_text).as_deref() {
        Some("google") => "google",
        Some("bing") => "bing",
        _ => "duckduckgo",
    }
}

fn build_search_url(query: &str, engine: Option<&str>) -> String {
    let encoded = percent_encode(query.trim());
    match search_engine_name(engine) {
        "google" => format!("https://www.google.com/search?q={encoded}"),
        "bing" => format!("https://www.bing.com/search?q={encoded}"),
        _ => format!("https://duckduckgo.com/?q={encoded}"),
    }
}

fn page_text_scope(page: &PageResearchData, scope: Option<&str>) -> (&'static str, String) {
    match scope.map(normalize_text).as_deref() {
        Some("main") => ("main", page.content_scopes.main_text.clone()),
        Some("article") => ("article", page.content_scopes.article_text.clone()),
        Some("controls") => ("controls", page.content_scopes.controls_text.clone()),
        Some("overlays") => ("overlays", page.content_scopes.overlay_text.clone()),
        _ => ("full", page.visible_text.clone()),
    }
}

fn render_page_markdown(page: &PageResearchData, scope: Option<&str>) -> String {
    let (scope_name, scoped_text) = page_text_scope(page, scope);
    let mut sections = Vec::new();
    if let Some(title) = page.title.as_ref()
        && !title.trim().is_empty()
    {
        sections.push(format!("# {}", title.trim()));
    }
    if let Some(url) = page.url.as_ref() {
        sections.push(format!("Source: {url}"));
    }
    if let Some(canonical_url) = page.canonical_url.as_ref()
        && page.url.as_ref() != Some(canonical_url)
    {
        sections.push(format!("Canonical: {canonical_url}"));
    }
    sections.push(format!("Scope: {scope_name}"));
    sections.push(format!("Page Type: {}", page.page_type));
    if !page.headings.is_empty() {
        sections.push(String::from("## Headings"));
        sections.extend(page.headings.iter().map(|heading| {
            let level = heading.level.unwrap_or(2).clamp(1, 6) as usize;
            format!("{} {}", "#".repeat(level), heading.text)
        }));
    }
    if !scoped_text.trim().is_empty() {
        sections.push(format!("## {} Text", scope_name.to_ascii_uppercase()));
        sections.push(scoped_text);
    }
    if !page.primary_controls.is_empty() {
        sections.push(String::from("## Primary Controls"));
        sections.extend(page.primary_controls.iter().map(|control| {
            let label = control
                .label
                .as_deref()
                .unwrap_or(control.text.as_str())
                .trim();
            let label = if label.is_empty() {
                control.kind.as_str()
            } else {
                label
            };
            format!(
                "- {} [{}]{}",
                label,
                control.kind,
                control
                    .placeholder
                    .as_ref()
                    .map(|placeholder| format!(" placeholder={placeholder}"))
                    .unwrap_or_default()
            )
        }));
    }
    if !page.primary_links.is_empty() {
        sections.push(String::from("## Primary Links"));
        sections.extend(page.primary_links.iter().map(|link| {
            let label = if link.text.trim().is_empty() {
                link.href.clone()
            } else {
                link.text.clone()
            };
            format!("- [{label}]({})", link.href)
        }));
    }
    if !page.suggested_next_actions.is_empty() {
        sections.push(String::from("## Suggested Next Actions"));
        sections.extend(
            page.suggested_next_actions
                .iter()
                .map(|action| format!("- {action}")),
        );
    }
    sections.join("\n\n")
}

fn page_text_response(page: &PageResearchData, scope: Option<&str>) -> PageTextResponse {
    let (scope_name, text) = page_text_scope(page, scope);
    PageTextResponse {
        scope: scope_name.to_string(),
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        text,
        page_type: page.page_type.clone(),
        useful_text_available: page.useful_text_available,
        interactive_elements_available: page.interactive_elements_available,
        blocked_by_overlay: page.blocked_by_overlay,
        blocker_signals: page.blocker_signals.clone(),
        suggested_next_actions: page.suggested_next_actions.clone(),
        likely_not_found: page.likely_not_found,
        not_found_signals: page.not_found_signals.clone(),
        suggested_search_query: page.suggested_search_query.clone(),
    }
}

fn page_markdown_response(page: &PageResearchData, scope: Option<&str>) -> PageMarkdownResponse {
    let (scope_name, _) = page_text_scope(page, scope);
    PageMarkdownResponse {
        scope: scope_name.to_string(),
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        markdown: render_page_markdown(page, scope),
        page_type: page.page_type.clone(),
        useful_text_available: page.useful_text_available,
        interactive_elements_available: page.interactive_elements_available,
        blocked_by_overlay: page.blocked_by_overlay,
        blocker_signals: page.blocker_signals.clone(),
        suggested_next_actions: page.suggested_next_actions.clone(),
        likely_not_found: page.likely_not_found,
        not_found_signals: page.not_found_signals.clone(),
        suggested_search_query: page.suggested_search_query.clone(),
    }
}

fn page_actions_response(page: &PageResearchData) -> PageActionsResponse {
    PageActionsResponse {
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        page_type: page.page_type.clone(),
        useful_text_available: page.useful_text_available,
        interactive_elements_available: page.interactive_elements_available,
        blocked_by_overlay: page.blocked_by_overlay,
        blocker_signals: page.blocker_signals.clone(),
        primary_links: page.primary_links.clone(),
        primary_controls: page.primary_controls.clone(),
        suggested_next_actions: page.suggested_next_actions.clone(),
        suggested_search_query: page.suggested_search_query.clone(),
    }
}

fn page_forms_response(page: &PageResearchData) -> PageFormsResponse {
    PageFormsResponse {
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        page_type: page.page_type.clone(),
        auth_wall_likely: page.auth_wall_likely,
        blocked_by_overlay: page.blocked_by_overlay,
        blocker_signals: page.blocker_signals.clone(),
        forms: page.forms.clone(),
        suggested_next_actions: page.suggested_next_actions.clone(),
    }
}

fn snippet_for_match(text: &str, normalized_query: &str) -> Option<String> {
    if normalized_query.is_empty() {
        return None;
    }
    let lower = text.to_lowercase();
    let start = lower.find(normalized_query)?;
    let end = start.saturating_add(normalized_query.len());
    let snippet_start = start.saturating_sub(80);
    let snippet_end = (end + 80).min(text.len());
    Some(text[snippet_start..snippet_end].trim().to_string())
}

fn page_find_matches(page: &PageResearchData, query: &str, exact: bool) -> Vec<PageFindMatch> {
    let normalized_query = normalize_text(query);
    if normalized_query.is_empty() {
        return Vec::new();
    }
    let mut matches = Vec::new();
    for heading in &page.headings {
        let candidate = normalize_text(&heading.text);
        let matched = if exact {
            candidate == normalized_query
        } else {
            candidate.contains(&normalized_query)
        };
        if matched {
            matches.push(PageFindMatch {
                kind: String::from("heading"),
                text: heading.text.clone(),
                level: heading.level,
                href: None,
                index: None,
                snippet: Some(heading.text.clone()),
            });
        }
    }
    for link in &page.links {
        let link_text = normalize_text(&link.text);
        let href_text = normalize_text(&link.href);
        let matched = if exact {
            link_text == normalized_query || href_text == normalized_query
        } else {
            link_text.contains(&normalized_query) || href_text.contains(&normalized_query)
        };
        if matched {
            matches.push(PageFindMatch {
                kind: String::from("link"),
                text: if link.text.trim().is_empty() {
                    link.href.clone()
                } else {
                    link.text.clone()
                },
                level: None,
                href: Some(link.href.clone()),
                index: Some(link.index),
                snippet: link.title.clone(),
            });
        }
    }
    for control in &page.controls {
        let control_text = normalize_text(
            &[
                control.label.as_deref().unwrap_or_default(),
                control.text.as_str(),
                control.placeholder.as_deref().unwrap_or_default(),
                control.role.as_deref().unwrap_or_default(),
            ]
            .join(" "),
        );
        let matched = if exact {
            control_text == normalized_query
        } else {
            control_text.contains(&normalized_query)
        };
        if matched {
            matches.push(PageFindMatch {
                kind: String::from("control"),
                text: control
                    .label
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| control.text.clone()),
                level: None,
                href: control.href.clone(),
                index: Some(control.index),
                snippet: control.placeholder.clone().or_else(|| {
                    if control.actions.is_empty() {
                        None
                    } else {
                        Some(control.actions.join(", "))
                    }
                }),
            });
        }
    }
    if let Some(snippet) = snippet_for_match(&page.visible_text, &normalized_query) {
        matches.push(PageFindMatch {
            kind: String::from("text"),
            text: query.to_string(),
            level: None,
            href: None,
            index: None,
            snippet: Some(snippet),
        });
    }
    matches
}

fn matching_links(
    page: &PageResearchData,
    text: &str,
    exact: bool,
    href_contains: Option<&str>,
) -> Vec<PageResearchLink> {
    let normalized_text = normalize_text(text);
    let normalized_href = href_contains.map(normalize_text);
    page.links
        .iter()
        .filter(|link| {
            let link_text = if link.text.trim().is_empty() {
                link.href.as_str()
            } else {
                link.text.as_str()
            };
            let link_text = normalize_text(link_text);
            let text_matches = if exact {
                link_text == normalized_text
            } else {
                link_text.contains(&normalized_text)
            };
            let href_matches = normalized_href
                .as_ref()
                .is_none_or(|needle| normalize_text(&link.href).contains(needle));
            text_matches && href_matches
        })
        .cloned()
        .collect()
}

fn origin_key(url: &str) -> String {
    let trimmed = url.trim();
    if let Some((scheme, rest)) = trimmed.split_once("://") {
        let host = rest.split('/').next().unwrap_or(rest);
        return format!("{scheme}://{host}");
    }
    trimmed.to_string()
}

fn publish_chrome_state(chrome_tx: &watch::Sender<BrowserChromeState>, state: BrowserChromeState) {
    let _ = chrome_tx.send_if_modified(|current| {
        if *current != state {
            *current = state.clone();
            true
        } else {
            false
        }
    });
}

fn publish_tabs_state(tabs_tx: &watch::Sender<BrowserUiState>, state: BrowserUiState) {
    let _ = tabs_tx.send_if_modified(|current| {
        if *current != state {
            *current = state.clone();
            true
        } else {
            false
        }
    });
}

fn publish_runtime_tab_state(
    tabs: &mut BrowserTabController,
    chrome_tx: &watch::Sender<BrowserChromeState>,
    tabs_tx: &watch::Sender<BrowserUiState>,
) {
    let active_id = tabs.active_tab_id();
    if let Ok(state) = tabs.refresh_tab_state(active_id) {
        publish_chrome_state(chrome_tx, state);
    }
    publish_tabs_state(tabs_tx, tabs.snapshot());
}

fn refresh_selected_tab_state(
    tabs: &mut BrowserTabController,
    tab_id: u64,
    chrome_tx: &watch::Sender<BrowserChromeState>,
    tabs_tx: &watch::Sender<BrowserUiState>,
) {
    if let Ok(state) = tabs.refresh_tab_state(tab_id)
        && tab_id == tabs.active_tab_id()
    {
        publish_chrome_state(chrome_tx, state);
    }
    publish_tabs_state(tabs_tx, tabs.snapshot());
}

pub fn router(state: ApiState) -> Router {
    use super::chrome;
    use super::ui;
    use tower_http::cors::CorsLayer;
    use tower_http::services::{ServeDir, ServeFile};

    let mut app = Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(readiness))
        .route("/doctor", get(doctor))
        .route("/runtime", get(runtime_info))
        .route("/telemetry", get(telemetry))
        .route("/session", post(inject_session).get(snapshot_session))
        .route("/session/save", post(save_session_profile))
        .route("/session/load", post(load_session_profile))
        .route("/search", post(search))
        .route("/navigate", post(navigate))
        .route("/execute", post(execute))
        .route("/page", get(page_research))
        .route("/page/text", get(page_text))
        .route("/page/markdown", get(page_markdown))
        .route("/page/actions", get(page_actions))
        .route("/page/forms", get(page_forms))
        .route("/page/links", get(page_links))
        .route("/page/headings", get(page_headings))
        .route("/page/find", post(page_find))
        .route("/page/open-link", post(page_open_link))
        .route("/dom", get(snapshot_dom))
        .route("/events", get(events))
        .route("/trace/enable", post(enable_trace))
        .route("/tabs", get(list_tabs).post(create_tab))
        .route("/tabs/:tab_id/activate", post(activate_tab))
        .route("/tabs/:tab_id/close", post(close_tab))
        .route("/ui/bootstrap", get(ui::dashboard_bootstrap))
        .route("/ui/vnc", get(ui::vnc_websocket))
        .route("/ui/chrome/state", get(chrome::chrome_state_sse))
        .route(
            "/ui/chrome/state/snapshot",
            get(chrome::chrome_state_snapshot),
        )
        .route("/ui/chrome/tabs", get(chrome::chrome_tabs_sse))
        .route(
            "/ui/chrome/tabs/snapshot",
            get(chrome::chrome_tabs_snapshot),
        )
        .route("/ui/chrome/back", post(chrome::chrome_back))
        .route("/ui/chrome/forward", post(chrome::chrome_forward))
        .route("/ui/chrome/reload", post(chrome::chrome_reload))
        .route("/ui/chrome/stop", post(chrome::chrome_stop))
        .route("/ui/chrome/navigate", post(chrome::chrome_navigate))
        .route("/ui/chrome/tabs/new", post(chrome::chrome_new_tab))
        .route(
            "/ui/chrome/tabs/activate",
            post(chrome::chrome_activate_tab),
        )
        .route("/ui/chrome/tabs/close", post(chrome::chrome_close_tab))
        .layer(CorsLayer::permissive());

    if let Some(web_ui_dist) = state
        .dashboard_bootstrap()
        .as_ref()
        .and_then(|_| web_ui_dist_path())
    {
        let index = web_ui_dist.join("index.html");
        if index.is_file() {
            app = app
                .nest_service("/assets", ServeDir::new(web_ui_dist.join("assets")))
                .route_service("/", ServeFile::new(index.clone()))
                .fallback_service(ServeFile::new(index));
        }
    }

    app.with_state(state)
}

fn requires_linux_dashboard(browser_config: &BrowserConfig) -> bool {
    cfg!(target_os = "linux") && browser_config.mode == crate::browser::BrowserMode::Headful
}

fn web_ui_dist_path() -> Option<PathBuf> {
    if let Ok(current_exe) = std::env::current_exe() {
        for ancestor in current_exe.ancestors() {
            let bundled = ancestor.join("share").join("web-ui");
            if bundled.is_dir() {
                return Some(bundled);
            }
        }
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("web-ui")
        .join("dist");
    path.is_dir().then_some(path)
}

fn resolve_web_ui_dist(required: bool) -> Result<Option<PathBuf>, AegisError> {
    let path = web_ui_dist_path();
    if required && path.is_none() {
        return Err(AegisError::Bridge(
            "web UI assets are missing; run `scripts/build-web-ui.sh` before starting headful Linux serve"
                .into(),
        ));
    }
    Ok(path)
}

fn dashboard_url(addr: SocketAddr) -> String {
    format!("http://{addr}/")
}

async fn health(State(state): State<ApiState>) -> Json<HealthResponse> {
    let diagnostics = read_diagnostics(&state.diagnostics);
    Json(HealthResponse {
        control_plane_up: true,
        runtime_state: diagnostics.state.clone(),
        command_ready: diagnostics.command_ready,
        bridge_healthy: diagnostics.bridge_healthy,
        browser_backend_healthy: diagnostics.browser_backend_healthy,
        active_operation: diagnostics.active_operation,
        last_failure: diagnostics.last_failure,
    })
}

#[derive(Debug, Serialize)]
struct RuntimeInfo {
    tab_id: u64,
    tabs: BrowserUiState,
    host_library: PathBuf,
    browser: BrowserConfig,
    diagnostics: RuntimeDiagnosticsResponse,
    startup: ServeStartupMetrics,
    profile: SessionProfileInfo,
}

async fn runtime_info(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
) -> Result<Json<RuntimeInfo>, ApiError> {
    let tabs = state.tabs_state_snapshot();
    let tab_id = query.tab_id.unwrap_or(tabs.active_tab_id);
    Ok(Json(RuntimeInfo {
        tab_id,
        tabs,
        host_library: state.host_library.clone(),
        browser: state.browser.clone(),
        diagnostics: read_diagnostics(&state.diagnostics),
        profile: state.profile.clone(),
        startup: state
            .startup
            .lock()
            .map(|metrics| metrics.clone())
            .unwrap_or(ServeStartupMetrics {
                client_connect_ms: 0,
                api_bind_ms: 0,
                total_ready_ms: 0,
            }),
    }))
}

async fn telemetry(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
) -> Result<Json<TelemetryResponse>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotTelemetry(query.tab_id, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_telemetry", &state.diagnostics, reply_rx).await??,
    ))
}

#[allow(clippy::too_many_arguments)]
fn snapshot_telemetry_response(
    state: &ApiState,
    startup: &Arc<Mutex<ServeStartupMetrics>>,
    diagnostics: &Arc<Mutex<ServeDiagnostics>>,
    profile_store: &SessionProfileStore,
    credential_store: &AegisSecretStore,
    tabs: &BrowserUiState,
    tab_id: u64,
    client: &mut LoadedAegisClient,
) -> Result<TelemetryResponse, AegisError> {
    let runtime = client.runtime_mut().snapshot_telemetry();
    let session = client.snapshot_session()?;
    let chrome = client.snapshot_chrome_state().unwrap_or_default();
    let credentials_settings = AegisConfigStore::detect()
        .and_then(|store| store.load_credentials_settings())
        .unwrap_or(CredentialsSettings {
            version: 1,
            auto_store: true,
        });
    let credential_entries = credential_store
        .load_profile_credentials(&profile_store.info().profile)
        .map_err(AegisError::Bridge)?;
    let config_store = AegisConfigStore::detect().ok();
    Ok(TelemetryResponse {
        tab_id,
        tabs: tabs.clone(),
        host_library: state.host_library.clone(),
        browser: state.browser.clone(),
        startup: startup
            .lock()
            .map(|metrics| metrics.clone())
            .unwrap_or(ServeStartupMetrics {
                client_connect_ms: 0,
                api_bind_ms: 0,
                total_ready_ms: 0,
            }),
        diagnostics: read_diagnostics(diagnostics),
        chrome,
        runtime,
        session: build_session_telemetry(profile_store.info(), session),
        credentials: build_credentials_telemetry(credentials_settings, credential_entries),
        settings: build_runtime_settings_telemetry(config_store.as_ref()),
        dashboard: build_dashboard_telemetry(state),
    })
}

fn build_session_telemetry(profile: SessionProfileInfo, session: SessionState) -> SessionTelemetry {
    let cookies = session
        .cookies
        .iter()
        .map(|cookie| SessionCookieTelemetry {
            name: cookie.name.clone(),
            domain: cookie.domain.clone(),
            path: cookie.path.clone(),
            expires_unix: cookie.expires_unix,
            secure: cookie.secure,
            http_only: cookie.http_only,
            value_bytes: cookie.value.len(),
        })
        .collect::<Vec<_>>();
    let local_storage = session
        .local_storage
        .iter()
        .map(|(key, value)| SessionStorageEntryTelemetry {
            key: key.clone(),
            value_bytes: value.len(),
        })
        .collect::<Vec<_>>();
    let session_storage = session
        .session_storage
        .iter()
        .map(|(key, value)| SessionStorageEntryTelemetry {
            key: key.clone(),
            value_bytes: value.len(),
        })
        .collect::<Vec<_>>();
    let network_overrides = session
        .network_overrides
        .iter()
        .map(|override_| NetworkOverrideTelemetry {
            header: override_.header.clone(),
            value_bytes: override_.value.len(),
        })
        .collect::<Vec<_>>();

    SessionTelemetry {
        profile,
        cookie_count: cookies.len(),
        cookies,
        local_storage_count: local_storage.len(),
        local_storage,
        session_storage_count: session_storage.len(),
        session_storage,
        network_override_count: network_overrides.len(),
        network_overrides,
    }
}

fn build_credentials_telemetry(
    settings: CredentialsSettings,
    entries: Vec<StoredCredentialEntry>,
) -> CredentialsTelemetry {
    let entries = entries
        .into_iter()
        .map(|entry| CredentialTelemetryEntry {
            origin: entry.origin,
            username: entry.username,
            username_field: entry.username_field,
            password_field: entry.password_field,
            form_label: entry.form_label,
            created_at_ms: entry.created_at_ms,
            updated_at_ms: entry.updated_at_ms,
        })
        .collect::<Vec<_>>();
    CredentialsTelemetry {
        settings,
        stored_credentials_count: entries.len(),
        entries,
    }
}

fn build_runtime_settings_telemetry(store: Option<&AegisConfigStore>) -> RuntimeSettingsTelemetry {
    let agent = store.and_then(|store| store.get("agent").ok().flatten());
    let runtime = store.and_then(|store| store.get("runtime").ok().flatten());
    RuntimeSettingsTelemetry {
        default_profile: agent
            .as_ref()
            .and_then(|value| value.get("default_profile"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        headless_persistent: runtime
            .as_ref()
            .and_then(|value| value.get("modes"))
            .and_then(|value| value.get("headless"))
            .and_then(|value| value.get("persistent"))
            .and_then(Value::as_bool),
        headful_persistent: runtime
            .as_ref()
            .and_then(|value| value.get("modes"))
            .and_then(|value| value.get("headful"))
            .and_then(|value| value.get("persistent"))
            .and_then(Value::as_bool),
    }
}

fn build_dashboard_telemetry(state: &ApiState) -> DashboardTelemetry {
    DashboardTelemetry {
        headful_dashboard: state.dashboard_bootstrap().is_some(),
        bootstrap: state.dashboard_bootstrap(),
        vnc_addr: state.vnc_addr().map(|addr| addr.to_string()),
        resolution: state
            .dashboard_bootstrap()
            .as_ref()
            .map(|_| DASHBOARD_RESOLUTION.to_string()),
    }
}

async fn readiness(
    State(state): State<ApiState>,
) -> Result<Json<RuntimeDiagnosticsResponse>, ApiError> {
    let diagnostics = read_diagnostics(&state.diagnostics);
    if diagnostics.command_ready {
        Ok(Json(diagnostics))
    } else {
        Err(ApiError::readiness(diagnostics))
    }
}

async fn doctor(State(state): State<ApiState>) -> Json<RuntimeDiagnosticsResponse> {
    Json(read_diagnostics(&state.diagnostics))
}

async fn save_session_profile(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SaveSessionProfile(query.tab_id, reply_tx))
        .map_err(channel_error)?;
    let profile = await_command("save_session_profile", &state.diagnostics, reply_rx).await??;
    Ok(Json(profile))
}

async fn load_session_profile(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
) -> Result<Json<SessionProfileInfo>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::LoadSessionProfile(query.tab_id, reply_tx))
        .map_err(channel_error)?;
    let profile = await_command("load_session_profile", &state.diagnostics, reply_rx).await??;
    Ok(Json(profile))
}

async fn inject_session(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
    Json(body): Json<SessionState>,
) -> Result<StatusCode, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::InjectSession(query.tab_id, body, reply_tx))
        .map_err(channel_error)?;
    await_command("inject_session", &state.diagnostics, reply_rx).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn snapshot_session(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
) -> Result<Json<SessionState>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotSession(query.tab_id, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_session", &state.diagnostics, reply_rx).await??,
    ))
}

async fn navigate(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
    Json(body): Json<NavigateBody>,
) -> Result<Json<Vec<SequencedEvent>>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Navigate(query.tab_id, body.url, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("navigate", &state.diagnostics, reply_rx).await??,
    ))
}

async fn execute(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
    Json(body): Json<ExecuteBody>,
) -> Result<Json<ExecutionReport>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Execute(query.tab_id, body.commands, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("execute", &state.diagnostics, reply_rx).await??,
    ))
}

async fn snapshot_page_research(
    state: &ApiState,
    tab_id: Option<u64>,
) -> Result<PageResearchData, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::PageResearch(tab_id, reply_tx))
        .map_err(channel_error)?;
    Ok(await_command("page_research", &state.diagnostics, reply_rx).await??)
}

async fn search(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
    Json(body): Json<SearchBody>,
) -> Result<Json<SearchResponse>, ApiError> {
    let request_query = body.query.trim().to_string();
    if request_query.is_empty() {
        return Err(ApiError::bad_request(
            "search query must not be empty",
            "invalid_search_query",
        ));
    }
    let url = build_search_url(&request_query, body.engine.as_deref());
    let engine = search_engine_name(body.engine.as_deref()).to_string();
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Navigate(query.tab_id, url.clone(), reply_tx))
        .map_err(channel_error)?;
    let events = await_command("search", &state.diagnostics, reply_rx).await??;
    Ok(Json(SearchResponse {
        engine,
        query: request_query,
        url,
        events,
    }))
}

async fn page_research(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
) -> Result<Json<PageResearchData>, ApiError> {
    Ok(Json(snapshot_page_research(&state, query.tab_id).await?))
}

async fn page_text(
    State(state): State<ApiState>,
    Query(query): Query<PageReadQuery>,
) -> Result<Json<PageTextResponse>, ApiError> {
    let page = snapshot_page_research(&state, query.tab_id).await?;
    Ok(Json(page_text_response(&page, query.scope.as_deref())))
}

async fn page_markdown(
    State(state): State<ApiState>,
    Query(query): Query<PageReadQuery>,
) -> Result<Json<PageMarkdownResponse>, ApiError> {
    let page = snapshot_page_research(&state, query.tab_id).await?;
    Ok(Json(page_markdown_response(&page, query.scope.as_deref())))
}

async fn page_links(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
) -> Result<Json<PageLinksResponse>, ApiError> {
    let page = snapshot_page_research(&state, query.tab_id).await?;
    Ok(Json(PageLinksResponse {
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        links: page.links.clone(),
        likely_not_found: page.likely_not_found,
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn page_headings(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
) -> Result<Json<PageHeadingsResponse>, ApiError> {
    let page = snapshot_page_research(&state, query.tab_id).await?;
    Ok(Json(PageHeadingsResponse {
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        headings: page.headings.clone(),
        likely_not_found: page.likely_not_found,
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn page_actions(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
) -> Result<Json<PageActionsResponse>, ApiError> {
    let page = snapshot_page_research(&state, query.tab_id).await?;
    Ok(Json(page_actions_response(&page)))
}

async fn page_forms(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
) -> Result<Json<PageFormsResponse>, ApiError> {
    let page = snapshot_page_research(&state, query.tab_id).await?;
    Ok(Json(page_forms_response(&page)))
}

async fn page_find(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
    Json(body): Json<PageFindBody>,
) -> Result<Json<PageFindResponse>, ApiError> {
    let page = snapshot_page_research(&state, query.tab_id).await?;
    let matches = page_find_matches(&page, &body.text, body.exact);
    Ok(Json(PageFindResponse {
        query: body.text,
        exact: body.exact,
        title: page.title.clone(),
        url: page.url.clone(),
        canonical_url: page.canonical_url.clone(),
        match_count: matches.len(),
        matches,
        likely_not_found: page.likely_not_found,
        suggested_search_query: page.suggested_search_query.clone(),
    }))
}

async fn page_open_link(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
    Json(body): Json<PageOpenLinkBody>,
) -> Result<Json<PageOpenLinkResponse>, ApiError> {
    let page = snapshot_page_research(&state, query.tab_id).await?;
    let candidates = matching_links(&page, &body.text, body.exact, body.href_contains.as_deref());
    if candidates.is_empty() {
        return Err(ApiError::bad_request(
            "no page links matched the requested text",
            "page_link_not_found",
        ));
    }
    let chosen = if let Some(index) = body.index {
        candidates.get(index).cloned().ok_or_else(|| {
            ApiError::bad_request(
                "page link index is out of range",
                "page_link_index_out_of_range",
            )
        })?
    } else if candidates.len() == 1 {
        candidates[0].clone()
    } else {
        return Err(ApiError::bad_request(
            "multiple page links matched; pass `index` to disambiguate",
            "page_link_ambiguous",
        ));
    };
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Navigate(
            query.tab_id,
            chosen.href.clone(),
            reply_tx,
        ))
        .map_err(channel_error)?;
    let events = await_command("page_open_link", &state.diagnostics, reply_rx).await??;
    Ok(Json(PageOpenLinkResponse {
        text: body.text,
        exact: body.exact,
        href_contains: body.href_contains,
        candidate_count: candidates.len(),
        chosen,
        events,
    }))
}

async fn snapshot_dom(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
) -> Result<Json<DomSnapshot>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::SnapshotDom(query.tab_id, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("snapshot_dom", &state.diagnostics, reply_rx).await??,
    ))
}

async fn events(
    State(state): State<ApiState>,
    Query(query): Query<EventQuery>,
) -> Result<Json<EventReadWindow>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::Events(query.tab_id, query.since, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("events", &state.diagnostics, reply_rx).await??,
    ))
}

async fn enable_trace(
    State(state): State<ApiState>,
    Query(query): Query<TabQuery>,
    Json(body): Json<TraceBody>,
) -> Result<StatusCode, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::EnableTrace(query.tab_id, body.path, reply_tx))
        .map_err(channel_error)?;
    await_command("enable_trace", &state.diagnostics, reply_rx).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_tabs(State(state): State<ApiState>) -> Result<Json<BrowserUiState>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::ListTabs(reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("list_tabs", &state.diagnostics, reply_rx).await??,
    ))
}

async fn create_tab(
    State(state): State<ApiState>,
    Json(body): Json<TabCreateBody>,
) -> Result<Json<TabOperationResponse>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::CreateTab(body, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("create_tab", &state.diagnostics, reply_rx).await??,
    ))
}

async fn activate_tab(
    State(state): State<ApiState>,
    AxumPath(tab_id): AxumPath<u64>,
) -> Result<Json<TabOperationResponse>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::ActivateTab(tab_id, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("activate_tab", &state.diagnostics, reply_rx).await??,
    ))
}

async fn close_tab(
    State(state): State<ApiState>,
    AxumPath(tab_id): AxumPath<u64>,
) -> Result<Json<BrowserUiState>, ApiError> {
    let (reply_tx, reply_rx) = oneshot::channel();
    state
        .tx
        .send(ApiCommand::CloseTab(tab_id, reply_tx))
        .map_err(channel_error)?;
    Ok(Json(
        await_command("close_tab", &state.diagnostics, reply_rx).await??,
    ))
}

fn channel_error(error: mpsc::SendError<ApiCommand>) -> ApiError {
    ApiError::from(AegisError::Bridge(error.to_string()))
}

struct ApiError {
    status: StatusCode,
    body: ApiErrorBody,
}

impl ApiError {
    fn bad_request(message: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            body: ApiErrorBody {
                error: message.into(),
                code: code.into(),
                operation: None,
                stage: None,
                elapsed_ms: None,
                timed_out: false,
                restart_recommended: false,
            },
        }
    }

    fn timeout(operation: &str) -> Self {
        Self {
            status: StatusCode::GATEWAY_TIMEOUT,
            body: ApiErrorBody {
                error: format!(
                    "operation `{operation}` exceeded the server timeout and the runtime is now marked wedged"
                ),
                code: "operation_timeout".into(),
                operation: Some(operation.to_string()),
                stage: Some("awaiting_control_plane_reply".into()),
                elapsed_ms: Some(COMMAND_TIMEOUT.as_millis() as u64),
                timed_out: true,
                restart_recommended: true,
            },
        }
    }

    fn readiness(diagnostics: RuntimeDiagnosticsResponse) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            body: ApiErrorBody {
                error: "runtime is not command-ready".into(),
                code: "not_ready".into(),
                operation: diagnostics
                    .active_operation
                    .as_ref()
                    .map(|op| op.name.clone()),
                stage: diagnostics
                    .active_operation
                    .as_ref()
                    .map(|op| op.stage.clone()),
                elapsed_ms: diagnostics
                    .active_operation
                    .as_ref()
                    .map(|op| op.elapsed_ms),
                timed_out: diagnostics
                    .active_operation
                    .as_ref()
                    .is_some_and(|op| op.timed_out),
                restart_recommended: diagnostics
                    .last_failure
                    .as_ref()
                    .is_some_and(|failure| failure.restart_recommended),
            },
        }
    }
}

impl From<AegisError> for ApiError {
    fn from(value: AegisError) -> Self {
        let message = value.to_string();
        if let Some(native) = parse_native_operation_error(&message) {
            return Self {
                status: if native.timed_out {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                },
                body: ApiErrorBody {
                    error: native.message,
                    code: "native_operation_error".into(),
                    operation: Some(native.operation),
                    stage: Some(native.stage),
                    elapsed_ms: Some(native.elapsed_ms),
                    timed_out: native.timed_out,
                    restart_recommended: native.restart_recommended,
                },
            };
        }

        let status = match value {
            AegisError::InvalidSession(_) => StatusCode::BAD_REQUEST,
            AegisError::Serialize(_)
            | AegisError::Deserialize(_)
            | AegisError::Io(_)
            | AegisError::Utf8(_)
            | AegisError::Protocol(_)
            | AegisError::Bridge(_) => StatusCode::BAD_GATEWAY,
        };
        Self {
            status,
            body: ApiErrorBody {
                error: message,
                code: "aegis_error".into(),
                operation: None,
                stage: None,
                elapsed_ms: None,
                timed_out: false,
                restart_recommended: false,
            },
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(self.body)).into_response()
    }
}

#[derive(Debug, Serialize)]
pub struct ApiErrorBody {
    pub error: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    pub timed_out: bool,
    pub restart_recommended: bool,
}

async fn await_command<T>(
    operation: &str,
    diagnostics: &Arc<Mutex<ServeDiagnostics>>,
    reply_rx: oneshot::Receiver<Result<T, AegisError>>,
) -> Result<Result<T, AegisError>, ApiError> {
    match timeout(COMMAND_TIMEOUT, reply_rx).await {
        Ok(result) => result.map_err(reply_error),
        Err(_) => {
            mark_operation_timeout(diagnostics, operation);
            Err(ApiError::timeout(operation))
        }
    }
}

fn reply_error(error: oneshot::error::RecvError) -> ApiError {
    ApiError::from(AegisError::Bridge(error.to_string()))
}

fn record_operation_started(
    diagnostics: &Arc<Mutex<ServeDiagnostics>>,
    operation: &str,
    stage: &str,
) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.begin_operation(operation, stage);
    }
}

fn record_operation_finished<T>(
    diagnostics: &Arc<Mutex<ServeDiagnostics>>,
    operation: &str,
    client: &LoadedAegisClient,
    result: &Result<T, AegisError>,
) {
    let runtime = client.runtime_status();
    match result {
        Ok(_) => {
            if let Ok(mut diagnostics) = diagnostics.lock() {
                diagnostics.complete_success(operation, runtime);
            }
        }
        Err(error) => record_operation_failure(
            diagnostics,
            operation,
            failure_from_error(operation, "native operation failed", error),
            Some(runtime),
        ),
    }
}

fn record_operation_failure(
    diagnostics: &Arc<Mutex<ServeDiagnostics>>,
    operation: &str,
    failure: FailureSnapshot,
    runtime: Option<RuntimeStatus>,
) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.complete_failure(operation, failure, runtime);
    }
}

fn record_heartbeat(diagnostics: &Arc<Mutex<ServeDiagnostics>>, client: &LoadedAegisClient) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.record_runtime_snapshot(client.runtime_status());
    }
}

fn mark_operation_timeout(diagnostics: &Arc<Mutex<ServeDiagnostics>>, operation: &str) {
    if let Ok(mut diagnostics) = diagnostics.lock() {
        diagnostics.mark_timeout(operation, COMMAND_TIMEOUT.as_millis() as u64);
    }
}

fn read_diagnostics(diagnostics: &Arc<Mutex<ServeDiagnostics>>) -> RuntimeDiagnosticsResponse {
    diagnostics
        .lock()
        .map(|diagnostics| diagnostics.snapshot())
        .unwrap_or_else(|_| {
            ServeDiagnostics::new(RuntimeStatus {
                bootstrapped: false,
                bootstrap_duration_ms: None,
                dom_nodes: 0,
                dom_snapshot_available: false,
                retained_event_count: 0,
                latest_event_sequence: 0,
                oldest_retained_event_sequence: None,
                current_url: None,
                current_title: None,
                document_ready_state: None,
                last_dom_refresh_at_ms: None,
                last_live_state_refresh_at_ms: None,
                last_event_at_ms: None,
                last_successful_command_at_ms: None,
                last_successful_bridge_roundtrip_at_ms: None,
            })
            .snapshot()
        })
}

fn failure_from_error(operation: &str, stage: &str, error: &AegisError) -> FailureSnapshot {
    if let Some(native) = parse_native_operation_error(&error.to_string()) {
        return FailureSnapshot {
            operation: native.operation,
            stage: native.stage,
            message: native.message,
            elapsed_ms: native.elapsed_ms,
            timed_out: native.timed_out,
            restart_recommended: native.restart_recommended,
            first_seen_at_ms: now_ms(),
            last_seen_at_ms: now_ms(),
        };
    }

    FailureSnapshot {
        operation: operation.to_string(),
        stage: stage.to_string(),
        message: error.to_string(),
        elapsed_ms: 0,
        timed_out: false,
        restart_recommended: false,
        first_seen_at_ms: now_ms(),
        last_seen_at_ms: now_ms(),
    }
}

fn parse_native_operation_error(message: &str) -> Option<NativeOperationError> {
    let payload = message.strip_prefix("bridge error: ").unwrap_or(message);
    let parsed: NativeOperationError = serde_json::from_str(payload).ok()?;
    (parsed.kind == "operation_error").then_some(parsed)
}

impl ServeDiagnostics {
    fn new(runtime: RuntimeStatus) -> Self {
        Self {
            runtime,
            active_operation: None,
            last_failure: None,
            total_operations: 0,
            successful_operations: 0,
            timed_out_operations: 0,
            next_operation_id: 1,
            recent_operations: VecDeque::with_capacity(RECENT_OPERATION_LIMIT),
            operation_aggregates: BTreeMap::new(),
        }
    }

    fn begin_operation(&mut self, name: &str, stage: &str) {
        self.total_operations += 1;
        self.active_operation = Some(ActiveOperation {
            id: self.next_operation_id,
            name: name.to_string(),
            stage: stage.to_string(),
            started_at_ms: now_ms(),
            started_at: Instant::now(),
            timed_out: false,
        });
        self.next_operation_id += 1;
    }

    fn complete_success(&mut self, name: &str, runtime: RuntimeStatus) {
        self.successful_operations += 1;
        self.runtime = runtime;
        if let Some(active) = self.active_operation.take() {
            let elapsed_ms = active.started_at.elapsed().as_millis() as u64;
            self.record_completed_operation(CompletedOperationSnapshot {
                id: active.id,
                name: active.name,
                stage: active.stage,
                status: "success".into(),
                started_at_ms: active.started_at_ms,
                finished_at_ms: now_ms(),
                elapsed_ms,
                timed_out: active.timed_out,
                error_message: None,
            });
            self.update_operation_aggregate(name, true, active.timed_out, elapsed_ms);
        }
        self.last_failure = None;
    }

    fn complete_failure(
        &mut self,
        name: &str,
        mut failure: FailureSnapshot,
        runtime: Option<RuntimeStatus>,
    ) {
        if let Some(runtime) = runtime {
            self.runtime = runtime;
        }
        if let Some(previous) = self.last_failure.as_ref() {
            failure.first_seen_at_ms = previous.first_seen_at_ms;
        }
        self.last_failure = Some(failure);
        if let Some(active) = self.active_operation.take() {
            let elapsed_ms = active.started_at.elapsed().as_millis() as u64;
            let error_message = self
                .last_failure
                .as_ref()
                .map(|failure| failure.message.clone());
            self.record_completed_operation(CompletedOperationSnapshot {
                id: active.id,
                name: active.name,
                stage: active.stage,
                status: "failure".into(),
                started_at_ms: active.started_at_ms,
                finished_at_ms: now_ms(),
                elapsed_ms,
                timed_out: active.timed_out,
                error_message,
            });
            self.update_operation_aggregate(name, false, active.timed_out, elapsed_ms);
        }
    }

    fn mark_timeout(&mut self, operation: &str, elapsed_ms: u64) {
        self.timed_out_operations += 1;
        if let Some(active) = self.active_operation.as_mut() {
            active.timed_out = true;
            active.stage = "awaiting_control_plane_reply".into();
        }
        let now = now_ms();
        self.last_failure = Some(FailureSnapshot {
            operation: operation.to_string(),
            stage: "awaiting_control_plane_reply".into(),
            message: "the API timed out waiting for the runtime owner thread to reply".into(),
            elapsed_ms,
            timed_out: true,
            restart_recommended: true,
            first_seen_at_ms: self
                .last_failure
                .as_ref()
                .map(|failure| failure.first_seen_at_ms)
                .unwrap_or(now),
            last_seen_at_ms: now,
        });
        if let Some(active) = self.active_operation.take() {
            self.record_completed_operation(CompletedOperationSnapshot {
                id: active.id,
                name: active.name,
                stage: active.stage,
                status: "timeout".into(),
                started_at_ms: active.started_at_ms,
                finished_at_ms: now,
                elapsed_ms,
                timed_out: true,
                error_message: self
                    .last_failure
                    .as_ref()
                    .map(|failure| failure.message.clone()),
            });
            self.update_operation_aggregate(operation, false, true, elapsed_ms);
        }
    }

    fn record_runtime_snapshot(&mut self, runtime: RuntimeStatus) {
        self.runtime = runtime;
    }

    fn record_completed_operation(&mut self, operation: CompletedOperationSnapshot) {
        self.recent_operations.push_front(operation);
        while self.recent_operations.len() > RECENT_OPERATION_LIMIT {
            let _ = self.recent_operations.pop_back();
        }
    }

    fn update_operation_aggregate(
        &mut self,
        name: &str,
        success: bool,
        timed_out: bool,
        elapsed_ms: u64,
    ) {
        let aggregate = self
            .operation_aggregates
            .entry(name.to_string())
            .or_default();
        aggregate.total_count += 1;
        if success {
            aggregate.success_count += 1;
        } else {
            aggregate.failure_count += 1;
        }
        if timed_out {
            aggregate.timeout_count += 1;
        }
        aggregate.total_elapsed_ms += elapsed_ms;
        aggregate.last_elapsed_ms = elapsed_ms;
        aggregate.min_elapsed_ms = if aggregate.min_elapsed_ms == 0 {
            elapsed_ms
        } else {
            aggregate.min_elapsed_ms.min(elapsed_ms)
        };
        aggregate.max_elapsed_ms = aggregate.max_elapsed_ms.max(elapsed_ms);
        match elapsed_ms {
            0..=49 => aggregate.histogram.lt_50_ms += 1,
            50..=99 => aggregate.histogram.lt_100_ms += 1,
            100..=249 => aggregate.histogram.lt_250_ms += 1,
            250..=499 => aggregate.histogram.lt_500_ms += 1,
            500..=999 => aggregate.histogram.lt_1000_ms += 1,
            _ => aggregate.histogram.gte_1000_ms += 1,
        }
    }

    fn snapshot(&self) -> RuntimeDiagnosticsResponse {
        let active_operation = self
            .active_operation
            .as_ref()
            .map(|operation| OperationSnapshot {
                id: operation.id,
                name: operation.name.clone(),
                stage: operation.stage.clone(),
                started_at_ms: operation.started_at_ms,
                elapsed_ms: operation.started_at.elapsed().as_millis() as u64,
                timed_out: operation.timed_out,
            });
        let state = if active_operation.as_ref().is_some_and(|op| op.timed_out) {
            RuntimeOperationalState::Wedged
        } else if active_operation.is_some() {
            RuntimeOperationalState::Busy
        } else if self
            .last_failure
            .as_ref()
            .is_some_and(|failure| failure.timed_out || failure.restart_recommended)
        {
            RuntimeOperationalState::Wedged
        } else if self.last_failure.is_some() {
            RuntimeOperationalState::Degraded
        } else if self
            .runtime
            .last_successful_bridge_roundtrip_at_ms
            .is_some()
        {
            RuntimeOperationalState::Ready
        } else {
            RuntimeOperationalState::Starting
        };
        let command_ready = !matches!(
            state,
            RuntimeOperationalState::Starting | RuntimeOperationalState::Wedged
        );
        RuntimeDiagnosticsResponse {
            state,
            control_plane_up: true,
            command_ready,
            bridge_healthy: self
                .runtime
                .last_successful_bridge_roundtrip_at_ms
                .is_some()
                && self
                    .last_failure
                    .as_ref()
                    .is_none_or(|failure| !failure.timed_out),
            browser_backend_healthy: self.runtime.bootstrapped
                && self
                    .last_failure
                    .as_ref()
                    .is_none_or(|failure| !failure.restart_recommended),
            dom_snapshot_available: self.runtime.dom_snapshot_available,
            active_operation,
            last_failure: self.last_failure.clone(),
            total_operations: self.total_operations,
            successful_operations: self.successful_operations,
            timed_out_operations: self.timed_out_operations,
            recent_operations: self.recent_operations.iter().cloned().collect(),
            operation_aggregates: self
                .operation_aggregates
                .iter()
                .map(|(name, aggregate)| OperationAggregateTelemetry {
                    name: name.clone(),
                    total_count: aggregate.total_count,
                    success_count: aggregate.success_count,
                    failure_count: aggregate.failure_count,
                    timeout_count: aggregate.timeout_count,
                    avg_elapsed_ms: if aggregate.total_count > 0 {
                        aggregate.total_elapsed_ms / aggregate.total_count
                    } else {
                        0
                    },
                    min_elapsed_ms: aggregate.min_elapsed_ms,
                    max_elapsed_ms: aggregate.max_elapsed_ms,
                    last_elapsed_ms: aggregate.last_elapsed_ms,
                    histogram: aggregate.histogram.clone(),
                })
                .collect(),
            runtime: self.runtime.clone(),
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::{default_true, origin_key};

    #[test]
    fn navigation_target_defaults_to_blank() {
        let navigation_target = |url: Option<&str>| {
            let trimmed = url.unwrap_or_default().trim();
            if trimmed.is_empty() {
                "about:blank".to_string()
            } else {
                trimmed.to_string()
            }
        };
        assert_eq!(navigation_target(None), "about:blank");
        assert_eq!(navigation_target(Some("   ")), "about:blank");
        assert_eq!(
            navigation_target(Some("https://example.com")),
            "https://example.com"
        );
    }

    #[test]
    fn origin_key_strips_paths() {
        assert_eq!(
            origin_key("https://example.com/docs/page"),
            "https://example.com"
        );
        assert_eq!(origin_key("about:blank"), "about:blank");
    }

    #[test]
    fn default_true_is_true() {
        assert!(default_true());
    }
}
