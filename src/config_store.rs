use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::{AegisStatePaths, with_state_file_lock, write_state_file};

#[derive(Debug, Clone)]
pub struct AegisConfigStore {
    paths: AegisStatePaths,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialsSettings {
    pub version: u32,
    pub auto_store: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCredentialEntry {
    pub origin: String,
    pub username: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_label: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialInput {
    pub origin: String,
    pub username: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredCredentialsPayload {
    version: u32,
    #[serde(default)]
    entries: Vec<StoredCredentialEntry>,
}

impl AegisConfigStore {
    pub fn detect() -> Result<Self, String> {
        Ok(Self {
            paths: AegisStatePaths::detect()?,
        })
    }

    pub fn get(&self, concern: &str) -> Result<Option<Value>, String> {
        validate_name(concern, "name")?;
        let path = self.paths.settings_file(concern);
        if !path.exists() {
            return Ok(None);
        }
        with_state_file_lock(&path, || {
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("failed to read config {}: {error}", path.display()))?;
            let value: Value = match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(_) => {
                    let default = default_config_payload(concern);
                    self.paths.repair_json_file(&path, &default, "config")?;
                    default
                }
            };
            Ok(Some(value))
        })
    }

    pub fn set(&self, concern: &str, value: &Value) -> Result<PathBuf, String> {
        validate_name(concern, "name")?;
        let path = self.paths.settings_file(concern);
        let payload = serde_json::to_vec_pretty(value)
            .map_err(|error| format!("failed to encode config {concern}: {error}"))?;
        with_state_file_lock(&path, || write_state_file(&path, &payload))?;
        Ok(path)
    }

    pub fn load_credentials_settings(&self) -> Result<CredentialsSettings, String> {
        let value = self
            .get("credentials")?
            .unwrap_or_else(default_credentials_settings_payload);
        Ok(CredentialsSettings {
            version: 1,
            auto_store: value
                .get("auto_store")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredProfileSecrets {
    version: u32,
    secrets: Value,
}

#[derive(Debug, Clone)]
pub struct AegisSecretStore {
    paths: AegisStatePaths,
}

impl AegisSecretStore {
    pub fn detect() -> Result<Self, String> {
        Ok(Self {
            paths: AegisStatePaths::detect()?,
        })
    }

    pub fn load_profile_secrets(&self, profile: &str) -> Result<Value, String> {
        validate_name(profile, "profile")?;
        self.paths.ensure_profile_layout(profile)?;
        let path = self.paths.profile_secrets_file(profile);
        if !path.exists() {
            return Ok(Value::Object(Default::default()));
        }
        with_state_file_lock(&path, || self.load_profile_secrets_unlocked(&path))
    }

    pub fn save_profile_secrets(&self, profile: &str, secrets: &Value) -> Result<PathBuf, String> {
        validate_name(profile, "profile")?;
        self.paths.ensure_profile_layout(profile)?;
        let path = self.paths.profile_secrets_file(profile);
        let payload = serde_json::to_vec_pretty(&StoredProfileSecrets {
            version: 1,
            secrets: secrets.clone(),
        })
        .map_err(|error| format!("failed to encode secret payload: {error}"))?;
        with_state_file_lock(&path, || write_state_file(&path, &payload))?;
        Ok(path)
    }

    pub fn load_profile_credentials(
        &self,
        profile: &str,
    ) -> Result<Vec<StoredCredentialEntry>, String> {
        let secrets = self.load_profile_secrets(profile)?;
        Ok(parse_credentials_entries(&secrets))
    }

    pub fn upsert_profile_credential(
        &self,
        profile: &str,
        input: CredentialInput,
    ) -> Result<(PathBuf, StoredCredentialEntry), String> {
        validate_credential_input(&input)?;
        self.paths.ensure_profile_layout(profile)?;
        let path = self.paths.profile_secrets_file(profile);
        with_state_file_lock(&path, || {
            let mut secrets = if path.exists() {
                self.load_profile_secrets_unlocked(&path)?
            } else {
                Value::Object(Default::default())
            };
            let mut entries = parse_credentials_entries(&secrets);
            let now_ms = unix_timestamp_ms();
            let mut entry = StoredCredentialEntry {
                origin: input.origin.trim().to_string(),
                username: input.username.trim().to_string(),
                password: input.password.clone(),
                username_field: normalize_optional(input.username_field.clone()),
                password_field: normalize_optional(input.password_field.clone()),
                form_label: normalize_optional(input.form_label.clone()),
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            };
            if let Some(existing) = entries.iter_mut().find(|existing| {
                existing.origin.eq_ignore_ascii_case(&entry.origin)
                    && existing.username.eq_ignore_ascii_case(&entry.username)
            }) {
                entry.created_at_ms = existing.created_at_ms;
                entry.updated_at_ms = now_ms;
                *existing = entry.clone();
            } else {
                entries.push(entry.clone());
                entries.sort_by(|left, right| {
                    left.origin
                        .cmp(&right.origin)
                        .then_with(|| left.username.cmp(&right.username))
                });
            }
            upsert_credentials_entries(&mut secrets, entries)?;
            let payload = serde_json::to_vec_pretty(&StoredProfileSecrets {
                version: 1,
                secrets: secrets.clone(),
            })
            .map_err(|error| format!("failed to encode secret payload: {error}"))?;
            write_state_file(&path, &payload)?;
            Ok((path.clone(), entry))
        })
    }

    pub fn remove_profile_credential(
        &self,
        profile: &str,
        origin: &str,
        username: &str,
    ) -> Result<(PathBuf, bool), String> {
        self.paths.ensure_profile_layout(profile)?;
        let path = self.paths.profile_secrets_file(profile);
        with_state_file_lock(&path, || {
            let mut secrets = if path.exists() {
                self.load_profile_secrets_unlocked(&path)?
            } else {
                Value::Object(Default::default())
            };
            let mut entries = parse_credentials_entries(&secrets);
            let before = entries.len();
            entries.retain(|entry| {
                !(entry.origin.eq_ignore_ascii_case(origin)
                    && entry.username.eq_ignore_ascii_case(username))
            });
            let removed = entries.len() != before;
            upsert_credentials_entries(&mut secrets, entries)?;
            let payload = serde_json::to_vec_pretty(&StoredProfileSecrets {
                version: 1,
                secrets: secrets.clone(),
            })
            .map_err(|error| format!("failed to encode secret payload: {error}"))?;
            write_state_file(&path, &payload)?;
            Ok((path.clone(), removed))
        })
    }

    pub fn clear_profile_credentials(&self, profile: &str) -> Result<PathBuf, String> {
        self.paths.ensure_profile_layout(profile)?;
        let path = self.paths.profile_secrets_file(profile);
        with_state_file_lock(&path, || {
            let mut secrets = if path.exists() {
                self.load_profile_secrets_unlocked(&path)?
            } else {
                Value::Object(Default::default())
            };
            upsert_credentials_entries(&mut secrets, Vec::new())?;
            let payload = serde_json::to_vec_pretty(&StoredProfileSecrets {
                version: 1,
                secrets: secrets.clone(),
            })
            .map_err(|error| format!("failed to encode secret payload: {error}"))?;
            write_state_file(&path, &payload)?;
            Ok(path.clone())
        })
    }

    fn load_profile_secrets_unlocked(&self, path: &std::path::Path) -> Result<Value, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("failed to read secret file {}: {error}", path.display()))?;
        let stored: StoredProfileSecrets = match serde_json::from_slice(&bytes) {
            Ok(stored) => stored,
            Err(_) => {
                let default = default_secrets_payload();
                self.paths
                    .repair_json_file(path, &default, "profile secrets")?;
                StoredProfileSecrets {
                    version: 1,
                    secrets: default["secrets"].clone(),
                }
            }
        };
        Ok(stored.secrets)
    }
}

fn validate_name(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!(
            "{label} {value:?} must use only letters, numbers, '.', '-', or '_'"
        ));
    }
    Ok(())
}

fn default_config_payload(concern: &str) -> Value {
    match concern {
        "agent" => serde_json::json!({
            "version": 1,
            "default_profile": "default"
        }),
        "runtime" => serde_json::json!({
            "version": 1,
            "bootstrap_page": "local",
            "modes": {
                "headless": {"persistent": true},
                "headful": {"persistent": true}
            }
        }),
        "credentials" => default_credentials_settings_payload(),
        _ => Value::Object(Default::default()),
    }
}

fn default_secrets_payload() -> Value {
    serde_json::json!({
        "version": 1,
        "secrets": {
            "credentials": {
                "version": 1,
                "entries": []
            }
        }
    })
}

fn default_credentials_settings_payload() -> Value {
    serde_json::json!({
        "version": 1,
        "auto_store": true
    })
}

fn parse_credentials_entries(secrets: &Value) -> Vec<StoredCredentialEntry> {
    secrets
        .get("credentials")
        .cloned()
        .and_then(|value| serde_json::from_value::<StoredCredentialsPayload>(value).ok())
        .map(|stored| stored.entries)
        .unwrap_or_default()
}

fn upsert_credentials_entries(
    secrets: &mut Value,
    entries: Vec<StoredCredentialEntry>,
) -> Result<(), String> {
    let object = secrets
        .as_object_mut()
        .ok_or_else(|| "secret payload must be a JSON object".to_string())?;
    object.insert(
        "credentials".to_string(),
        serde_json::to_value(StoredCredentialsPayload {
            version: 1,
            entries,
        })
        .map_err(|error| format!("failed to encode credentials payload: {error}"))?,
    );
    Ok(())
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn validate_credential_input(input: &CredentialInput) -> Result<(), String> {
    if input.origin.trim().is_empty() {
        return Err("credential origin must not be empty".into());
    }
    if input.username.trim().is_empty() {
        return Err("credential username must not be empty".into());
    }
    if input.password.is_empty() {
        return Err("credential password must not be empty".into());
    }
    Ok(())
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::{
        AegisConfigStore, AegisSecretStore, CredentialInput, CredentialsSettings, validate_name,
    };
    use crate::state::aegis_test_env_lock;
    use std::fs;

    #[test]
    fn validate_name_rejects_invalid_characters() {
        assert!(validate_name("profile-1", "profile").is_ok());
        assert!(validate_name("bad/name", "profile").is_err());
    }

    #[test]
    fn config_get_repairs_corrupt_default_file() {
        let _guard = aegis_test_env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().expect("temporary state dir should be created");
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        let store = AegisConfigStore::detect().expect("config store should initialize");
        let path = store.paths.settings_file("agent");
        fs::write(&path, b"{bad-json").expect("corrupt config fixture should be written");
        let value = store
            .get("agent")
            .expect("config lookup should repair corrupt file")
            .expect("agent config should exist");
        assert_eq!(value["default_profile"], "default");
        let backups = fs::read_dir(path.parent().expect("agent config should have a parent"))
            .expect("agent config directory should be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("agent.json.corrupt."))
            .count();
        assert_eq!(backups, 1);
        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }

    #[test]
    fn secrets_get_repairs_corrupt_profile_file() {
        let _guard = aegis_test_env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().expect("temporary state dir should be created");
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        let store = AegisSecretStore::detect().expect("secret store should initialize");
        let path = store.paths.profile_secrets_file("default");
        fs::write(&path, b"{bad-json").expect("corrupt secret fixture should be written");
        let value = store
            .load_profile_secrets("default")
            .expect("secret lookup should repair corrupt file");
        assert_eq!(
            value,
            serde_json::json!({
                "credentials": {
                    "version": 1,
                    "entries": []
                }
            })
        );
        let backups = fs::read_dir(path.parent().expect("secret file should have a parent"))
            .expect("secret directory should be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("secrets.json.corrupt."))
            .count();
        assert_eq!(backups, 1);
        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }

    #[test]
    fn credentials_settings_default_to_auto_store() {
        let _guard = aegis_test_env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().expect("temporary state dir should be created");
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        let store = AegisConfigStore::detect().expect("config store should initialize");
        let settings = store
            .load_credentials_settings()
            .expect("credentials settings should load");
        assert_eq!(
            settings,
            CredentialsSettings {
                version: 1,
                auto_store: true
            }
        );
        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }

    #[test]
    fn credential_store_round_trips_and_preserves_other_secrets() {
        let _guard = aegis_test_env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().expect("temporary state dir should be created");
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        let store = AegisSecretStore::detect().expect("secret store should initialize");
        store
            .save_profile_secrets(
                "default",
                &serde_json::json!({
                    "api_keys": {"openai": "secret"}
                }),
            )
            .expect("profile secrets should be saved");
        let (_, saved) = store
            .upsert_profile_credential(
                "default",
                CredentialInput {
                    origin: "https://example.com".into(),
                    username: "saint".into(),
                    password: "pw".into(),
                    username_field: Some("email".into()),
                    password_field: Some("password".into()),
                    form_label: Some("Sign in".into()),
                },
            )
            .expect("credential should be upserted");
        assert_eq!(saved.origin, "https://example.com");
        let credentials = store
            .load_profile_credentials("default")
            .expect("credentials should load");
        assert_eq!(credentials.len(), 1);
        assert_eq!(credentials[0].username, "saint");
        let raw = store
            .load_profile_secrets("default")
            .expect("raw secrets should load");
        assert_eq!(raw["api_keys"]["openai"], "secret");
        let (_, removed) = store
            .remove_profile_credential("default", "https://example.com", "saint")
            .expect("credential should be removed");
        assert!(removed);
        assert!(
            store
                .load_profile_credentials("default")
                .expect("credentials should load after removal")
                .is_empty()
        );
        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }

    #[test]
    fn secret_store_bootstraps_non_default_profile_layout_before_locking() {
        let _guard = aegis_test_env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let temp = tempfile::tempdir().expect("temporary state dir should be created");
        unsafe {
            std::env::set_var("AEGIS_HOME", temp.path());
        }
        let store = AegisSecretStore::detect().expect("secret store should initialize");
        let path = store
            .save_profile_secrets("shopify-clean", &serde_json::json!({}))
            .expect("profile layout should be created before locking");
        assert!(path.exists());
        assert!(temp.path().join("secrets/profiles/shopify-clean").is_dir());
        unsafe {
            std::env::remove_var("AEGIS_HOME");
        }
    }
}
