pub mod api;
pub mod browser;
pub mod client;
pub mod commands;
pub mod config_store;
pub mod dom;
pub mod display;
pub mod events;
pub mod host;
pub mod native;
pub mod runtime;
pub mod session;
pub mod state;
pub mod trace;
pub mod transport;

pub use api::server;
pub use browser::{BrowserConfig, BrowserMode};
pub use client::AegisClient;
pub use commands::command::{Command, CommandMatcher, CommandResult, CommandTarget, NodeId};
pub use config_store::{
    AegisConfigStore, AegisSecretStore, CredentialInput, CredentialsSettings, StoredCredentialEntry,
};
pub use dom::node::{DomNode, DomSnapshot};
pub use events::stream::{EventReadWindow, EventStream, EventType, RuntimeEvent, SequencedEvent};
pub use native::{
    NativeArch, NativeConfiguration, NativeDoctor, NativePlatform, NativeStatus, app_executable,
    artifact_for_target, build_native, configure_native,
};
pub use runtime::executor::{
    AegisRuntime, DomTelemetrySummary, EventTelemetrySummary, ExecutionReport,
    PageJsHeapTelemetry, PageNavigationTelemetry, PageResourceTelemetry, PageRuntimeTelemetry,
    PageViewportTelemetry, RecentLogTelemetry, RecentNavigationTelemetry,
    RecentNetworkRequestTelemetry, RuntimeTelemetrySnapshot, TraceTelemetry,
};
pub use session::cookies::{Cookie, SessionState};
pub use session::profile::{SessionProfileInfo, SessionProfileStore};
pub use session::storage::{NetworkOverride, StorageArea};
pub use state::AegisStatePaths;
pub use trace::recorder::TraceRecorder;
pub use trace::replayer::{ReplayState, replay_trace};
pub use transport::bridge::{
    AegisError, BatchRequest, BatchResponse, BridgeEventEnvelope, BrowserChromeState, CefBridge,
    HostApi, HostBuffer, HostFunctionTable, HostHandle, HostStatus,
};
pub use transport::protocol::{
    BatchWireResponse, EvalJsRequest, EvalJsResponse, EventsResponse, MessageEnvelope, MessageKind,
    NavigateRequest, NavigateResponse, TraceBatchRecord, TraceEventRecord, TraceFile,
    decode_message, encode_message,
};
