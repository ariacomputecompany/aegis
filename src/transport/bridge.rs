use std::ffi::c_void;
use std::ptr::NonNull;
use std::slice;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::commands::command::{Command, CommandResult};
use crate::dom::node::DomSnapshot;
use crate::events::stream::RuntimeEvent;
use crate::session::cookies::SessionState;
use crate::transport::protocol::{
    BatchWireResponse, EvalJsRequest, EvalJsResponse, EventsResponse, MessageKind, NavigateRequest,
    NavigateResponse, decode_message, encode_message,
};

const MAX_HOST_BUFFER_LEN: usize = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum AegisError {
    #[error("serialization error: {0}")]
    Serialize(serde_json::Error),
    #[error("deserialization error: {0}")]
    Deserialize(serde_json::Error),
    #[error("bridge error: {0}")]
    Bridge(String),
    #[error("invalid session: {0}")]
    InvalidSession(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("utf8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("protocol error: {0}")]
    Protocol(String),
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStatus {
    Ok = 0,
    Error = 1,
}

pub type HostHandle = *mut c_void;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct HostBuffer {
    pub ptr: *mut u8,
    pub len: usize,
}

impl HostBuffer {
    pub const fn empty() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            len: 0,
        }
    }
}

pub type HostApi = unsafe extern "C" fn(
    ctx: HostHandle,
    input_ptr: *const u8,
    input_len: usize,
    output: *mut HostBuffer,
) -> HostStatus;

pub type HostFree = unsafe extern "C" fn(ctx: HostHandle, buffer: HostBuffer);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostFunctionTable {
    pub install_runtime: HostApi,
    pub eval_js: HostApi,
    pub send_batch: HostApi,
    pub snapshot_dom: HostApi,
    pub inject_session: HostApi,
    pub snapshot_session: HostApi,
    pub drain_events: HostApi,
    pub navigate: HostApi,
    pub pump: HostApi,
    pub snapshot_chrome_state: HostApi,
    pub go_back: HostApi,
    pub go_forward: HostApi,
    pub reload_page: HostApi,
    pub stop_load: HostApi,
    pub free_buffer: HostFree,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BrowserChromeState {
    pub title: String,
    pub url: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub is_loading: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BridgeEventEnvelope {
    pub event: RuntimeEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchRequest {
    pub batch_id: u64,
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchResponse {
    pub batch_id: u64,
    pub results: Vec<CommandResult>,
    pub snapshot: Option<DomSnapshot>,
    #[serde(default)]
    pub events: Vec<BridgeEventEnvelope>,
}

enum BridgeBackend {
    Dynamic {
        host: NonNull<c_void>,
        fns: HostFunctionTable,
    },
}

pub struct CefBridge {
    backend: BridgeBackend,
}

impl CefBridge {
    pub fn new(host: HostHandle, fns: HostFunctionTable) -> Result<Self, AegisError> {
        let host =
            NonNull::new(host).ok_or_else(|| AegisError::Bridge("host handle is null".into()))?;
        Ok(Self {
            backend: BridgeBackend::Dynamic { host, fns },
        })
    }

    pub fn install_runtime(&mut self) -> Result<(), AegisError> {
        let payload = encode_message(MessageKind::InstallRuntime, &())?;
        let _response = self.invoke_message(MessageKind::InstallRuntime, &payload)?;
        Ok(())
    }

    pub fn eval_js(&mut self, script: &str) -> Result<String, AegisError> {
        let payload = encode_message(
            MessageKind::EvalJs,
            &EvalJsRequest {
                script: script.into(),
            },
        )?;
        let response = self.invoke_message(MessageKind::EvalJs, &payload)?;
        let response: EvalJsResponse = decode_message(MessageKind::EvalJs, &response)?;
        String::from_utf8(response.value).map_err(|error| AegisError::Bridge(error.to_string()))
    }

    pub fn send_batch(&mut self, request: &BatchRequest) -> Result<BatchResponse, AegisError> {
        let payload = encode_message(MessageKind::SendBatch, request)?;
        let response = self.invoke_message(MessageKind::SendBatch, &payload)?;
        let response: BatchWireResponse = decode_message(MessageKind::SendBatch, &response)?;
        Ok(BatchResponse {
            batch_id: response.batch_id,
            results: response.results,
            snapshot: response.snapshot,
            events: response.events,
        })
    }

    pub fn snapshot_dom(&mut self) -> Result<DomSnapshot, AegisError> {
        let payload = encode_message(MessageKind::SnapshotDom, &())?;
        let response = self.invoke_message(MessageKind::SnapshotDom, &payload)?;
        decode_message(MessageKind::SnapshotDom, &response)
    }

    pub fn inject_session(&mut self, session: SessionState) -> Result<(), AegisError> {
        let payload = encode_message(MessageKind::InjectSession, &session)?;
        let _response = self.invoke_message(MessageKind::InjectSession, &payload)?;
        Ok(())
    }

    pub fn snapshot_session(&mut self) -> Result<SessionState, AegisError> {
        let payload = encode_message(MessageKind::SnapshotSession, &())?;
        let response = self.invoke_message(MessageKind::SnapshotSession, &payload)?;
        decode_message(MessageKind::SnapshotSession, &response)
    }

    pub fn drain_events(&mut self) -> Result<Vec<BridgeEventEnvelope>, AegisError> {
        let payload = encode_message(MessageKind::DrainEvents, &())?;
        let response = self.invoke_message(MessageKind::DrainEvents, &payload)?;
        if response.is_empty() {
            return Ok(Vec::new());
        }
        let response: EventsResponse = decode_message(MessageKind::DrainEvents, &response)?;
        Ok(response.events)
    }

    pub fn navigate(&mut self, url: &str) -> Result<BatchResponse, AegisError> {
        let payload = encode_message(MessageKind::Navigate, &NavigateRequest { url: url.into() })?;
        let response = self.invoke_message(MessageKind::Navigate, &payload)?;
        let response: NavigateResponse = decode_message(MessageKind::Navigate, &response)?;
        Ok(BatchResponse {
            batch_id: 0,
            results: Vec::new(),
            snapshot: response.snapshot,
            events: response.events,
        })
    }

    pub fn pump(&mut self) -> Result<(), AegisError> {
        match &mut self.backend {
            BridgeBackend::Dynamic { host, fns } => {
                let _ = Self::invoke_raw(*host, *fns, fns.pump, &[])?;
                Ok(())
            }
        }
    }

    pub fn snapshot_chrome_state(&mut self) -> Result<BrowserChromeState, AegisError> {
        match &mut self.backend {
            BridgeBackend::Dynamic { host, fns } => {
                let raw = Self::invoke_raw(*host, *fns, fns.snapshot_chrome_state, &[])?;
                if raw.is_empty() {
                    return Ok(BrowserChromeState::default());
                }
                serde_json::from_slice(&raw).map_err(AegisError::Deserialize)
            }
        }
    }

    pub fn go_back(&mut self) -> Result<(), AegisError> {
        match &mut self.backend {
            BridgeBackend::Dynamic { host, fns } => {
                let _ = Self::invoke_raw(*host, *fns, fns.go_back, &[])?;
                Ok(())
            }
        }
    }

    pub fn go_forward(&mut self) -> Result<(), AegisError> {
        match &mut self.backend {
            BridgeBackend::Dynamic { host, fns } => {
                let _ = Self::invoke_raw(*host, *fns, fns.go_forward, &[])?;
                Ok(())
            }
        }
    }

    pub fn reload_page(&mut self) -> Result<(), AegisError> {
        match &mut self.backend {
            BridgeBackend::Dynamic { host, fns } => {
                let _ = Self::invoke_raw(*host, *fns, fns.reload_page, &[])?;
                Ok(())
            }
        }
    }

    pub fn stop_load(&mut self) -> Result<(), AegisError> {
        match &mut self.backend {
            BridgeBackend::Dynamic { host, fns } => {
                let _ = Self::invoke_raw(*host, *fns, fns.stop_load, &[])?;
                Ok(())
            }
        }
    }

    fn invoke_message(&mut self, kind: MessageKind, input: &[u8]) -> Result<Vec<u8>, AegisError> {
        match &mut self.backend {
            BridgeBackend::Dynamic { host, fns } => {
                let function = match kind {
                    MessageKind::InstallRuntime => fns.install_runtime,
                    MessageKind::EvalJs => fns.eval_js,
                    MessageKind::SendBatch => fns.send_batch,
                    MessageKind::SnapshotDom => fns.snapshot_dom,
                    MessageKind::InjectSession => fns.inject_session,
                    MessageKind::SnapshotSession => fns.snapshot_session,
                    MessageKind::DrainEvents => fns.drain_events,
                    MessageKind::Navigate => fns.navigate,
                };
                Self::invoke_raw(*host, *fns, function, input)
            }
        }
    }

    fn invoke_raw(
        host: NonNull<c_void>,
        fns: HostFunctionTable,
        function: HostApi,
        input: &[u8],
    ) -> Result<Vec<u8>, AegisError> {
        let mut output = HostBuffer::empty();
        let status = unsafe {
            function(
                host.as_ptr(),
                input.as_ptr(),
                input.len(),
                &mut output as *mut HostBuffer,
            )
        };

        match status {
            HostStatus::Ok => Self::decode_buffer(host, fns, output),
            HostStatus::Error => {
                let message = Self::decode_buffer(host, fns, output)?;
                let message = String::from_utf8_lossy(&message).to_string();
                Err(AegisError::Bridge(message))
            }
        }
    }

    fn decode_buffer(
        host: NonNull<c_void>,
        fns: HostFunctionTable,
        buffer: HostBuffer,
    ) -> Result<Vec<u8>, AegisError> {
        if buffer.ptr.is_null() && buffer.len == 0 {
            return Ok(Vec::new());
        }
        if buffer.ptr.is_null() || buffer.len == 0 {
            return Err(AegisError::Bridge(
                "native host returned an invalid buffer shape".into(),
            ));
        }
        if buffer.len > MAX_HOST_BUFFER_LEN {
            unsafe {
                (fns.free_buffer)(host.as_ptr(), buffer);
            }
            return Err(AegisError::Bridge(format!(
                "native host returned an oversized buffer: {} bytes",
                buffer.len
            )));
        }

        let output = unsafe { slice::from_raw_parts(buffer.ptr.cast_const(), buffer.len) }.to_vec();
        unsafe {
            (fns.free_buffer)(host.as_ptr(), buffer);
        }
        Ok(output)
    }
}
