use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dom::diff::DomMutation;

const DEFAULT_MAX_RETAINED_EVENTS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    DomMutation,
    Navigation,
    Network,
    Log,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEvent {
    DomMutation {
        changes: Vec<DomMutation>,
    },
    Navigation {
        url: String,
    },
    Network {
        request_id: String,
        url: String,
        method: Option<String>,
        status_code: Option<u16>,
        status_text: Option<String>,
        mime_type: Option<String>,
        request_status: Option<String>,
        content_length_bytes: Option<i64>,
        received_content_length_bytes: Option<i64>,
        duration_ms: Option<u64>,
        redirect_url: Option<String>,
        error_code: Option<i32>,
        error_text: Option<String>,
        is_main_frame: Option<bool>,
        response_headers: Option<BTreeMap<String, String>>,
    },
    Log {
        level: String,
        message: String,
        data: Option<Value>,
    },
}

impl RuntimeEvent {
    pub fn event_type(&self) -> EventType {
        match self {
            RuntimeEvent::DomMutation { .. } => EventType::DomMutation,
            RuntimeEvent::Navigation { .. } => EventType::Navigation,
            RuntimeEvent::Network { .. } => EventType::Network,
            RuntimeEvent::Log { .. } => EventType::Log,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequencedEvent {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub event: RuntimeEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventReadWindow {
    pub requested_since: u64,
    pub oldest_available_sequence: Option<u64>,
    pub latest_sequence: u64,
    pub gap_detected: bool,
    pub events: Vec<SequencedEvent>,
}

#[derive(Debug, Clone)]
pub struct EventStream {
    events: VecDeque<SequencedEvent>,
    max_retained: Option<usize>,
}

impl Default for EventStream {
    fn default() -> Self {
        Self::with_max_retained(DEFAULT_MAX_RETAINED_EVENTS)
    }
}

impl EventStream {
    pub fn with_max_retained(max_retained: usize) -> Self {
        Self {
            events: VecDeque::new(),
            max_retained: Some(max_retained.max(1)),
        }
    }

    pub fn unbounded() -> Self {
        Self {
            events: VecDeque::new(),
            max_retained: None,
        }
    }

    pub fn push(&mut self, event: SequencedEvent) {
        self.events.push_back(event);
        self.trim_to_limit();
    }

    pub fn push_all(&mut self, events: Vec<SequencedEvent>) {
        self.events.extend(events);
        self.trim_to_limit();
    }

    pub fn read_from(&self, sequence: u64, filter: Option<EventType>) -> EventReadWindow {
        let oldest_available_sequence = self.oldest_sequence();
        let latest_sequence = self.latest_sequence();
        let gap_detected = oldest_available_sequence
            .is_some_and(|oldest| latest_sequence > 0 && sequence.saturating_add(1) < oldest);
        let events = self
            .events
            .iter()
            .filter(|entry| entry.sequence > sequence)
            .filter(|entry| filter.is_none_or(|kind| entry.event.event_type() == kind))
            .cloned()
            .collect();
        EventReadWindow {
            requested_since: sequence,
            oldest_available_sequence,
            latest_sequence,
            gap_detected,
            events,
        }
    }

    pub fn latest_sequence(&self) -> u64 {
        self.events.back().map(|event| event.sequence).unwrap_or(0)
    }

    pub fn oldest_sequence(&self) -> Option<u64> {
        self.events.front().map(|event| event.sequence)
    }

    pub fn retained_len(&self) -> usize {
        self.events.len()
    }

    fn trim_to_limit(&mut self) {
        let Some(max_retained) = self.max_retained else {
            return;
        };
        while self.events.len() > max_retained {
            let _ = self.events.pop_front();
        }
    }
}
