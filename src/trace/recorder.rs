use std::fs;
use std::path::{Path, PathBuf};

use crate::browser::BrowserConfig;
use crate::events::stream::SequencedEvent;
use crate::session::cookies::SessionState;
use crate::transport::bridge::{AegisError, BatchRequest, BatchResponse};
use crate::transport::protocol::{
    BatchWireResponse, TraceBatchRecord, TraceEventRecord, TraceFile,
};

#[derive(Debug, Clone)]
pub struct TraceRecorder {
    path: PathBuf,
    trace: TraceFile,
}

impl TraceRecorder {
    pub fn new(path: impl Into<PathBuf>, browser_config: BrowserConfig) -> Self {
        Self {
            path: path.into(),
            trace: TraceFile {
                protocol_version: 1,
                browser_config,
                initial_session: None,
                batches: Vec::new(),
            },
        }
    }

    pub fn set_initial_session(&mut self, session: SessionState) {
        self.trace.initial_session = Some(session);
    }

    pub fn record_batch(
        &mut self,
        request: BatchRequest,
        response: BatchResponse,
        emitted_events: &[SequencedEvent],
    ) {
        self.trace.batches.push(TraceBatchRecord {
            batch_id: request.batch_id,
            request,
            response: BatchWireResponse {
                batch_id: response.batch_id,
                results: response.results,
                snapshot: response.snapshot,
                events: response.events,
            },
            emitted_events: emitted_events
                .iter()
                .map(|event| TraceEventRecord {
                    sequence: event.sequence,
                    timestamp_ms: event.timestamp_ms,
                    event: event.event.clone(),
                })
                .collect(),
        });
    }

    pub fn flush(&self) -> Result<(), AegisError> {
        let bytes = serde_json::to_vec_pretty(&self.trace).map_err(AegisError::Serialize)?;
        fs::write(&self.path, bytes)?;
        Ok(())
    }

    pub fn load(path: impl Into<PathBuf>) -> Result<Self, AegisError> {
        let path = path.into();
        let bytes = fs::read(&path)?;
        let trace = serde_json::from_slice(&bytes).map_err(AegisError::Deserialize)?;
        Ok(Self { path, trace })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn batch_count(&self) -> usize {
        self.trace.batches.len()
    }

    pub fn has_initial_session(&self) -> bool {
        self.trace.initial_session.is_some()
    }

    pub fn trace(&self) -> &TraceFile {
        &self.trace
    }
}
