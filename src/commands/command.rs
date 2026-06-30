use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type NodeId = u64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandMatcher {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actionable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
#[serde(untagged)]
pub enum CommandTarget {
    Id {
        id: NodeId,
    },
    Match {
        #[serde(rename = "match")]
        matcher: CommandMatcher,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Click {
        #[serde(flatten)]
        target: CommandTarget,
    },
    Hover {
        #[serde(flatten)]
        target: CommandTarget,
    },
    SetValue {
        #[serde(flatten)]
        target: CommandTarget,
        value: String,
    },
    PressKey {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<CommandTarget>,
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(default)]
        alt_key: bool,
        #[serde(default)]
        ctrl_key: bool,
        #[serde(default)]
        meta_key: bool,
        #[serde(default)]
        shift_key: bool,
    },
    WaitFor {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<CommandTarget>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url_contains: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title_contains: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ready_state: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        poll_interval_ms: Option<u64>,
    },
    Scroll {
        x: i64,
        y: i64,
    },
    Eval {
        code: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CommandResult {
    pub fn ok(value: impl Into<Value>) -> Self {
        Self {
            ok: true,
            value: Some(value.into()),
            error: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            value: None,
            error: Some(message.into()),
        }
    }
}
