use std::ffi::CStr;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::time::Instant;

use libloading::{Library, Symbol};
use serde_json::to_vec;

use crate::browser::BrowserConfig;
use crate::client::AegisClient;
use crate::commands::command::Command;
use crate::dom::node::DomSnapshot;
use crate::events::stream::{EventReadWindow, SequencedEvent};
use crate::runtime::executor::{ExecutionReport, PageResearchData, RuntimeStatus};
use crate::session::cookies::SessionState;
use crate::transport::bridge::{AegisError, CefBridge, HostFunctionTable, HostHandle};
use crate::transport::protocol::HostRuntimeState;

type CreateHost = unsafe extern "C" fn(input_ptr: *const u8, input_len: usize) -> HostHandle;
type LastErrorMessage = unsafe extern "C" fn() -> *const std::ffi::c_char;
type DestroyHost = unsafe extern "C" fn(HostHandle);
type GetFunctionTable = unsafe extern "C" fn() -> HostFunctionTable;

pub struct LoadedHost {
    _library: Library,
    handle: HostHandle,
    destroy: DestroyHost,
    table: HostFunctionTable,
}

impl LoadedHost {
    pub fn open(path: impl AsRef<Path>, config: &BrowserConfig) -> Result<Self, AegisError> {
        let library = unsafe { Library::new(path.as_ref()) }
            .map_err(|error| AegisError::Bridge(error.to_string()))?;

        let create = {
            let symbol: Symbol<'_, CreateHost> = unsafe { library.get(b"aegis_create_host") }
                .map_err(|error| AegisError::Bridge(error.to_string()))?;
            *symbol
        };
        let destroy = {
            let symbol: Symbol<'_, DestroyHost> = unsafe { library.get(b"aegis_destroy_host") }
                .map_err(|error| AegisError::Bridge(error.to_string()))?;
            *symbol
        };
        let last_error = {
            let symbol: Symbol<'_, LastErrorMessage> =
                unsafe { library.get(b"aegis_last_error_message") }
                    .map_err(|error| AegisError::Bridge(error.to_string()))?;
            *symbol
        };
        let table = {
            let symbol: Symbol<'_, GetFunctionTable> =
                unsafe { library.get(b"aegis_get_function_table") }
                    .map_err(|error| AegisError::Bridge(error.to_string()))?;
            unsafe { symbol() }
        };

        let config_bytes = to_vec(config).map_err(AegisError::Serialize)?;
        let handle = unsafe { create(config_bytes.as_ptr(), config_bytes.len()) };
        if handle.is_null() {
            let message = unsafe { last_error() };
            let message = if message.is_null() {
                "native host returned null handle".to_string()
            } else {
                unsafe { CStr::from_ptr(message) }
                    .to_string_lossy()
                    .trim()
                    .to_string()
            };
            return Err(AegisError::Bridge(message));
        }

        Ok(Self {
            _library: library,
            handle,
            destroy,
            table,
        })
    }

    pub fn bridge(&self) -> Result<CefBridge, AegisError> {
        CefBridge::new(self.handle, self.table)
    }
}

impl Drop for LoadedHost {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)(self.handle);
        }
    }
}

#[derive(Clone, Copy)]
pub struct RuntimeCancelHandle {
    handle: HostHandle,
    request_cancel: unsafe extern "C" fn(HostHandle),
}

impl RuntimeCancelHandle {
    pub fn request_cancel(&self) {
        unsafe { (self.request_cancel)(self.handle) }
    }
}

unsafe impl Send for RuntimeCancelHandle {}
unsafe impl Sync for RuntimeCancelHandle {}

pub struct LoadedAegisClient {
    _host: LoadedHost,
    client: AegisClient,
    cancel_handle: RuntimeCancelHandle,
}

impl LoadedAegisClient {
    pub fn connect(path: impl AsRef<Path>, config: BrowserConfig) -> Result<Self, AegisError> {
        let started = Instant::now();
        let host = LoadedHost::open(path, &config)?;
        let bridge = host.bridge()?;
        let bootstrap_duration_ms = Some(started.elapsed().as_millis() as u64);
        let mut client = AegisClient::connect(bridge, config, bootstrap_duration_ms)?;
        let _ = client.runtime_mut().snapshot_host_state()?;
        let _ = client.runtime_mut().pump();
        let cancel_handle = RuntimeCancelHandle {
            handle: host.handle,
            request_cancel: host.table.request_cancel,
        };
        Ok(Self {
            _host: host,
            client,
            cancel_handle,
        })
    }

    pub fn navigate(&mut self, url: impl Into<String>) -> Result<Vec<SequencedEvent>, AegisError> {
        self.client.navigate(url)
    }

    pub fn execute(&mut self, commands: &[Command]) -> Result<ExecutionReport, AegisError> {
        self.client.execute(commands)
    }

    pub fn inject_session(&mut self, session: SessionState) -> Result<(), AegisError> {
        self.client.inject_session(session)
    }

    pub fn snapshot_session(&mut self) -> Result<SessionState, AegisError> {
        self.client.runtime_mut().snapshot_session()
    }

    pub fn snapshot_dom(&mut self) -> Result<DomSnapshot, AegisError> {
        self.client.runtime_mut().snapshot_dom()
    }

    pub fn page_research_data(&mut self) -> Result<PageResearchData, AegisError> {
        self.client.runtime_mut().page_research_data()
    }

    pub fn pump(&mut self) -> Result<(), AegisError> {
        self.client.pump()
    }

    pub fn enable_trace_recording(&mut self, path: impl Into<std::path::PathBuf>) {
        self.client.runtime_mut().enable_trace_recording(path);
    }

    pub fn events_since(&mut self, sequence: u64) -> Result<EventReadWindow, AegisError> {
        self.client.runtime_mut().enable_network_event_capture();
        let _ = self.client.runtime_mut().drain_pending_events()?;
        Ok(self.client.runtime().read_events_from(sequence))
    }

    pub fn browser_config(&self) -> &BrowserConfig {
        self.client.browser_config()
    }

    pub fn runtime_status(&self) -> RuntimeStatus {
        self.client.runtime().runtime_status()
    }

    pub fn snapshot_host_state(&mut self) -> Result<HostRuntimeState, AegisError> {
        self.client.runtime_mut().snapshot_host_state()
    }

    pub fn activate_browser(&mut self, browser_id: i32) -> Result<HostRuntimeState, AegisError> {
        self.client.runtime_mut().activate_browser(browser_id)
    }

    pub fn request_cancel(&self) {
        self.cancel_handle.request_cancel();
    }

    pub fn cancel_handle(&self) -> RuntimeCancelHandle {
        self.cancel_handle
    }
}

impl Deref for LoadedAegisClient {
    type Target = AegisClient;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl DerefMut for LoadedAegisClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.client
    }
}
