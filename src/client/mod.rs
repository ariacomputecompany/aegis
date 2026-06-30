use crate::browser::BrowserConfig;
use crate::commands::command::Command;
use crate::events::stream::{EventReadWindow, EventType, SequencedEvent};
use crate::runtime::executor::{AegisRuntime, ExecutionReport, PageResearchData};
use crate::session::cookies::SessionState;
use crate::transport::bridge::{AegisError, CefBridge};

pub struct AegisClient {
    runtime: AegisRuntime,
}

impl AegisClient {
    pub fn connect(
        bridge: CefBridge,
        browser_config: BrowserConfig,
        bootstrap_duration_ms: Option<u64>,
    ) -> Result<Self, AegisError> {
        Ok(Self {
            runtime: AegisRuntime::new(bridge, browser_config, bootstrap_duration_ms)?,
        })
    }

    pub fn navigate(&mut self, url: impl Into<String>) -> Result<Vec<SequencedEvent>, AegisError> {
        self.runtime.navigate(url.into())
    }

    pub fn execute(&mut self, commands: &[Command]) -> Result<ExecutionReport, AegisError> {
        self.runtime.execute(commands)
    }

    pub fn inject_session(&mut self, session: SessionState) -> Result<(), AegisError> {
        self.runtime.inject_session(session)
    }

    pub fn pump(&mut self) -> Result<(), AegisError> {
        self.runtime.pump()
    }

    pub fn page_research_data(&mut self) -> Result<PageResearchData, AegisError> {
        self.runtime.page_research_data()
    }

    pub fn navigation_events_since(&self, sequence: u64) -> EventReadWindow {
        self.runtime
            .event_stream()
            .read_from(sequence, Some(EventType::Navigation))
    }

    pub fn runtime(&self) -> &AegisRuntime {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut AegisRuntime {
        &mut self.runtime
    }

    pub fn request_cancel(&self) {
        self.runtime.request_cancel();
    }

    pub fn browser_config(&self) -> &BrowserConfig {
        self.runtime.browser_config()
    }
}
