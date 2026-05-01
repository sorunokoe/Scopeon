use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use url::Url;

// TODO: OTel OTLP export is planned via a future `scopeon-otel` crate.
// The `[telemetry]` config section has been removed until that crate is ready.

/// Per-model pricing override set by the user.
///
/// All fields are optional; omit any field to keep the built-in value.
///
/// # Example (`~/.scopeon/config.toml`)
/// ```toml
/// [pricing.overrides."claude-opus-4-5"]
/// input = 5.00
/// output = 25.00
/// cache_write = 6.25
/// cache_read = 0.50
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelPricingOverride {
    /// Override for input tokens in USD per million tokens.
    pub input: Option<f64>,
    /// Override for output tokens in USD per million tokens.
    pub output: Option<f64>,
    /// Override for cache-write tokens in USD per million tokens.
    pub cache_write: Option<f64>,
    /// Override for cache-read tokens in USD per million tokens.
    pub cache_read: Option<f64>,
}

/// User-defined pricing overrides keyed by model name prefix.
/// The key must start with the model prefix (e.g. `"claude-opus-4-5"`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PricingConfig {
    /// Map from model prefix → per-field overrides.
    #[serde(default)]
    pub overrides: HashMap<String, ModelPricingOverride>,
}

impl PricingConfig {
    /// Returns `true` if the user has defined at least one pricing override with
    /// at least one non-None field. An entry with all-None fields (e.g., a TOML
    /// section header with no values) is a no-op and should not trigger a reprice.
    /// §5.4: Previously this returned true for any non-empty map, causing the full
    /// reprice_all_in_transaction to run on every startup for empty override entries.
    pub fn has_overrides(&self) -> bool {
        self.overrides.values().any(|o| {
            o.input.is_some()
                || o.output.is_some()
                || o.cache_write.is_some()
                || o.cache_read.is_some()
        })
    }
}

/// Storage and data-retention settings.
///
/// TRIZ D5: Resolves NE-E (unbounded DB growth). Opt-in auto-purge keeps the
/// database lean without data loss risk — disabled by default.
///
/// # Example (`~/.scopeon/config.toml`)
/// ```toml
/// [storage]
/// # Automatically delete turn data older than N days on startup. Default: disabled.
/// retain_days = 90
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageConfig {
    /// Delete turns (and orphaned sessions) older than this many days on startup.
    /// `None` (default) = keep all data indefinitely.
    pub retain_days: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Daily budget in USD (0.0 = no limit)
    pub daily_usd: f64,
    /// Weekly budget in USD (0.0 = no limit)
    pub weekly_usd: f64,
    /// Monthly budget in USD (0.0 = no limit)
    pub monthly_usd: f64,
}

impl Default for BudgetConfig {
    fn default() -> Self {
        BudgetConfig {
            daily_usd: 0.0,
            weekly_usd: 0.0,
            monthly_usd: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OptimizerConfig {
    /// Last preset Scopeon applied per provider, keyed by provider id.
    #[serde(default)]
    pub applied_presets: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    pub general: GeneralConfig,
    pub providers: ProvidersConfig,
    pub dashboard: DashboardConfig,
    pub alerts: AlertsConfig,
    pub budget: BudgetConfig,
    #[serde(default)]
    pub optimizer: OptimizerConfig,
    #[serde(default)]
    pub pricing: PricingConfig,
    /// Storage and data-retention settings.
    #[serde(default)]
    pub storage: StorageConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub theme: String,
    pub refresh_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvidersConfig {
    pub enabled: Vec<String>,
    pub generic_paths: Vec<String>,
    pub generic_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardConfig {
    pub live_metrics: Vec<String>,
    pub insights_metrics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// HTTP(S) URL to POST alert payloads to.
    pub url: String,
    /// Which event types to fire for. Empty = all events.
    /// Valid values: "context_crisis", "context_warning", "budget_warning",
    /// "low_turns_left", "compaction_detected"
    #[serde(default)]
    pub events: Vec<String>,
}

impl WebhookConfig {
    /// Return the webhook URL with any embedded credentials removed.
    pub fn redacted_url(&self) -> String {
        redact_webhook_url(&self.url)
    }
}

/// Return a webhook URL safe for logs by stripping embedded credentials.
pub fn redact_webhook_url(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut parsed) => {
            let had_credentials = !parsed.username().is_empty() || parsed.password().is_some();
            if had_credentials {
                let _ = parsed.set_username("");
                let _ = parsed.set_password(None);
            }
            parsed.to_string()
        },
        Err(_) => url.to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertsConfig {
    pub daily_cost_usd: Option<f64>,
    pub cache_hit_rate_min: Option<f64>,
    /// Outgoing webhook endpoints. Each entry receives a POST with a JSON body
    /// containing the alert payload whenever the configured events fire.
    /// Example config.toml:
    ///
    /// ```toml
    /// [[alerts.webhooks]]
    /// url = "https://hooks.slack.com/services/T.../B.../..."
    /// events = ["context_crisis", "budget_warning"]
    /// ```
    #[serde(default)]
    pub webhooks: Vec<WebhookConfig>,
}

impl Default for UserConfig {
    fn default() -> Self {
        UserConfig {
            general: GeneralConfig {
                theme: "standard".to_string(),
                refresh_interval_secs: 2,
            },
            providers: ProvidersConfig {
                enabled: vec!["claude-code".to_string()],
                generic_paths: Vec::new(),
                generic_name: "Custom OpenAI".to_string(),
            },
            dashboard: DashboardConfig {
                live_metrics: vec![
                    "cache_hit_rate".to_string(),
                    "cost_per_turn".to_string(),
                    "token_velocity".to_string(),
                    "cache_roi".to_string(),
                    "thinking_ratio".to_string(),
                    "context_efficiency".to_string(),
                ],
                insights_metrics: Vec::new(),
            },
            alerts: AlertsConfig {
                daily_cost_usd: None,
                cache_hit_rate_min: None,
                webhooks: Vec::new(),
            },
            budget: BudgetConfig::default(),
            optimizer: OptimizerConfig::default(),
            pricing: PricingConfig::default(),
            storage: StorageConfig::default(),
        }
    }
}

impl UserConfig {
    pub fn config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".scopeon").join("config.toml"))
    }

    pub fn load() -> Self {
        let path = Self::config_path().filter(|p| p.exists());
        if let Some(path) = path {
            match std::fs::read_to_string(&path).and_then(|s| {
                toml::from_str::<UserConfig>(&s)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            }) {
                Ok(mut cfg) => {
                    let warnings = cfg.validate_and_fix();
                    for w in &warnings {
                        tracing::warn!("Config validation: {}", w);
                    }
                    return cfg;
                },
                Err(e) => {
                    tracing::warn!("Failed to parse config at {:?}: {}", path, e);
                },
            }
        }
        UserConfig::default()
    }

    /// Validate configuration values and fix dangerous values in-place.
    ///
    /// Returns a list of human-readable warning messages for each corrected value.
    /// These can be surfaced to the user as toasts on startup.
    pub fn validate_and_fix(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();

        // retain_days = 0 would delete ALL data on startup.
        if self.storage.retain_days == Some(0) {
            self.storage.retain_days = None;
            warnings.push(
                "config: retain_days = 0 would delete all data — disabled (use retain_days = 30 or similar)".into(),
            );
        }

        // Negative budget values make the UI permanently red and can cause division issues.
        if self.budget.daily_usd < 0.0 {
            warnings.push(format!(
                "config: daily_usd = {:.2} is negative — set to 0 (no limit)",
                self.budget.daily_usd
            ));
            self.budget.daily_usd = 0.0;
        }
        if self.budget.weekly_usd < 0.0 {
            warnings.push(format!(
                "config: weekly_usd = {:.2} is negative — set to 0 (no limit)",
                self.budget.weekly_usd
            ));
            self.budget.weekly_usd = 0.0;
        }
        if self.budget.monthly_usd < 0.0 {
            warnings.push(format!(
                "config: monthly_usd = {:.2} is negative — set to 0 (no limit)",
                self.budget.monthly_usd
            ));
            self.budget.monthly_usd = 0.0;
        }

        // refresh_interval_secs = 0 would spin the CPU.
        if self.general.refresh_interval_secs == 0 {
            warnings.push("config: refresh_interval_secs = 0 — set to 1".into());
            self.general.refresh_interval_secs = 1;
        }

        // Clamp alert threshold to a sensible range.
        if let Some(rate) = self.alerts.cache_hit_rate_min {
            if !(0.0..=100.0).contains(&rate) {
                warnings.push(format!(
                    "config: cache_hit_rate_min = {:.1} is outside 0–100 — clamped",
                    rate
                ));
                self.alerts.cache_hit_rate_min = Some(rate.clamp(0.0, 100.0));
            }
        }

        // Validate webhook URLs early so the runtime never shells out to unsupported
        // schemes or logs credential-bearing URLs verbatim.
        let mut valid_webhooks = Vec::with_capacity(self.alerts.webhooks.len());
        for mut webhook in std::mem::take(&mut self.alerts.webhooks) {
            let trimmed = webhook.url.trim().to_string();
            if trimmed.is_empty() {
                warnings.push("config: webhook URL is empty — disabled".into());
                continue;
            }
            webhook.url = trimmed;

            let parsed = match Url::parse(&webhook.url) {
                Ok(url) => url,
                Err(err) => {
                    warnings.push(format!(
                        "config: webhook URL '{}' is invalid ({err}) — disabled",
                        webhook.redacted_url()
                    ));
                    continue;
                },
            };

            match parsed.scheme() {
                "http" | "https" => {},
                scheme => {
                    warnings.push(format!(
                        "config: webhook URL '{}' uses unsupported scheme '{}' — disabled",
                        webhook.redacted_url(),
                        scheme
                    ));
                    continue;
                },
            }

            if parsed.host_str().is_none() {
                warnings.push(format!(
                    "config: webhook URL '{}' is missing a host — disabled",
                    webhook.redacted_url()
                ));
                continue;
            }

            if !parsed.username().is_empty() || parsed.password().is_some() {
                warnings.push(format!(
                    "config: webhook URL '{}' embeds credentials — keep config.toml private",
                    webhook.redacted_url()
                ));
            }

            if parsed.scheme() == "http" {
                warnings.push(format!(
                    "config: webhook URL '{}' uses cleartext HTTP — prefer HTTPS or a trusted local relay",
                    webhook.redacted_url()
                ));
            }

            valid_webhooks.push(webhook);
        }
        self.alerts.webhooks = valid_webhooks;

        warnings
    }

    pub fn save(&self) -> Result<()> {
        let path =
            Self::config_path().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let toml_str = toml::to_string_pretty(self)?;
        std::fs::write(&path, toml_str)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_webhook_url_strips_credentials() {
        let redacted = redact_webhook_url("https://user:secret@example.com/hooks/test");
        assert_eq!(redacted, "https://example.com/hooks/test");
    }

    #[test]
    fn validate_and_fix_drops_unsupported_webhook_schemes() {
        let mut cfg = UserConfig::default();
        cfg.alerts.webhooks = vec![
            WebhookConfig {
                url: "ftp://example.com/hook".into(),
                events: Vec::new(),
            },
            WebhookConfig {
                url: "https://example.com/hook".into(),
                events: Vec::new(),
            },
        ];

        let warnings = cfg.validate_and_fix();

        assert_eq!(cfg.alerts.webhooks.len(), 1);
        assert_eq!(cfg.alerts.webhooks[0].url, "https://example.com/hook");
        assert!(warnings
            .iter()
            .any(|w| w.contains("unsupported scheme 'ftp'")));
    }

    #[test]
    fn validate_and_fix_warns_on_http_and_redacts_credentials() {
        let mut cfg = UserConfig::default();
        cfg.alerts.webhooks = vec![WebhookConfig {
            url: "http://user:secret@example.com/hook".into(),
            events: Vec::new(),
        }];

        let warnings = cfg.validate_and_fix();

        assert_eq!(cfg.alerts.webhooks.len(), 1);
        assert!(warnings.iter().any(|w| w.contains("cleartext HTTP")));
        assert!(warnings.iter().any(|w| w.contains("embeds credentials")));
        assert!(warnings.iter().all(|w| !w.contains("secret")));
    }
}
