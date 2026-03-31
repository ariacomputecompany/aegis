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
use crate::runtime::executor::{ExecutionReport, RuntimeStatus};
use crate::session::cookies::SessionState;
use crate::transport::bridge::{AegisError, BrowserChromeState, CefBridge, HostFunctionTable, HostHandle};

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

pub struct LoadedAegisClient {
    _host: LoadedHost,
    client: AegisClient,
}

impl LoadedAegisClient {
    pub fn connect(path: impl AsRef<Path>, config: BrowserConfig) -> Result<Self, AegisError> {
        let started = Instant::now();
        let host = LoadedHost::open(path, &config)?;
        let bridge = host.bridge()?;
        let bootstrap_duration_ms = Some(started.elapsed().as_millis() as u64);
        let mut client = AegisClient::connect(bridge, config, bootstrap_duration_ms)?;
        client.runtime_mut().establish_command_bridge()?;
        Ok(Self {
            _host: host,
            client,
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

    pub fn pump(&mut self) -> Result<(), AegisError> {
        self.client.pump()
    }

    pub fn enable_trace_recording(&mut self, path: impl Into<std::path::PathBuf>) {
        self.client.runtime_mut().enable_trace_recording(path);
    }

    pub fn events_since(&mut self, sequence: u64) -> Result<EventReadWindow, AegisError> {
        let _ = self.client.runtime_mut().drain_pending_events()?;
        Ok(self.client.runtime().read_events_from(sequence))
    }

    pub fn browser_config(&self) -> &BrowserConfig {
        self.client.browser_config()
    }

    pub fn runtime_status(&self) -> RuntimeStatus {
        self.client.runtime().runtime_status()
    }

    pub fn snapshot_chrome_state(&mut self) -> Result<BrowserChromeState, AegisError> {
        self.client.runtime_mut().bridge_mut().snapshot_chrome_state()
    }

    pub fn go_back(&mut self) -> Result<(), AegisError> {
        self.client.runtime_mut().bridge_mut().go_back()
    }

    pub fn go_forward(&mut self) -> Result<(), AegisError> {
        self.client.runtime_mut().bridge_mut().go_forward()
    }

    pub fn reload_page(&mut self) -> Result<(), AegisError> {
        self.client.runtime_mut().bridge_mut().reload_page()
    }

    pub fn stop_load(&mut self) -> Result<(), AegisError> {
        self.client.runtime_mut().bridge_mut().stop_load()
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
