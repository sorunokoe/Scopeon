use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use toml::value::Table;

use crate::user_config::UserConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OptimizationProviderId {
    ClaudeCode,
    CopilotCli,
    Codex,
    GeminiCli,
}

impl OptimizationProviderId {
    pub fn all() -> [Self; 4] {
        [
            Self::ClaudeCode,
            Self::CopilotCli,
            Self::Codex,
            Self::GeminiCli,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::CopilotCli => "copilot-cli",
            Self::Codex => "codex",
            Self::GeminiCli => "gemini-cli",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::CopilotCli => "GitHub Copilot CLI",
            Self::Codex => "OpenAI Codex CLI",
            Self::GeminiCli => "Gemini CLI",
        }
    }

    pub fn from_alias(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "claude" | "claude-code" => Some(Self::ClaudeCode),
            "copilot" | "copilot-cli" | "github-copilot-cli" => Some(Self::CopilotCli),
            "codex" | "codex-cli" | "openai-codex-cli" => Some(Self::Codex),
            "gemini" | "gemini-cli" => Some(Self::GeminiCli),
            _ => None,
        }
    }
}

impl fmt::Display for OptimizationProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OptimizationPresetId {
    MostSavings,
    Balanced,
    MostSpeed,
    MostPower,
}

impl OptimizationPresetId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MostSavings => "most-savings",
            Self::Balanced => "balanced",
            Self::MostSpeed => "most-speed",
            Self::MostPower => "most-power",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::MostSavings => "Most savings",
            Self::Balanced => "Balanced",
            Self::MostSpeed => "Most speed",
            Self::MostPower => "Most power",
        }
    }

    pub fn all() -> [Self; 4] {
        [
            Self::MostSavings,
            Self::Balanced,
            Self::MostSpeed,
            Self::MostPower,
        ]
    }

    pub fn from_alias(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "most-savings" | "savings" => Some(Self::MostSavings),
            "balanced" | "balance" => Some(Self::Balanced),
            "most-speed" | "speed" => Some(Self::MostSpeed),
            "most-power" | "power" => Some(Self::MostPower),
            _ => None,
        }
    }
}

impl fmt::Display for OptimizationPresetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationSupport {
    ConfigAndLaunch,
    LaunchOnly,
    ExplainOnly,
}

impl OptimizationSupport {
    pub fn label(self) -> &'static str {
        match self {
            Self::ConfigAndLaunch => "config + launcher",
            Self::LaunchOnly => "launcher",
            Self::ExplainOnly => "explain only",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationPreset {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub tradeoff: String,
    pub config_strategy: String,
    pub command_preview: String,
    pub optimizations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderOptimizationReport {
    pub provider_id: String,
    pub provider_name: String,
    pub detected: bool,
    pub support: OptimizationSupport,
    pub config_path: Option<String>,
    pub launcher_dir: String,
    pub current_preset: Option<String>,
    pub summary: String,
    pub notes: Vec<String>,
    pub docs: Vec<String>,
    pub presets: Vec<OptimizationPreset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileArtifactPreview {
    pub kind: String,
    pub path: String,
    pub description: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetPreview {
    pub provider_id: String,
    pub provider_name: String,
    pub preset_id: String,
    pub support: OptimizationSupport,
    pub launcher_path: String,
    pub launch_command: String,
    pub artifacts: Vec<FileArtifactPreview>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyReport {
    pub provider_id: String,
    pub provider_name: String,
    pub preset_id: String,
    pub support: OptimizationSupport,
    pub launcher_path: String,
    pub launch_command: String,
    pub files_written: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn list_provider_optimization_reports(
    user_config: &UserConfig,
) -> Vec<ProviderOptimizationReport> {
    OptimizationProviderId::all()
        .into_iter()
        .map(|provider| provider_report(provider, user_config))
        .collect()
}

pub fn provider_report(
    provider: OptimizationProviderId,
    user_config: &UserConfig,
) -> ProviderOptimizationReport {
    let detected = provider_detected(provider);
    let current_preset = user_config
        .optimizer
        .applied_presets
        .get(provider.as_str())
        .cloned();
    let launcher_dir = launcher_dir()
        .unwrap_or_else(|_| PathBuf::from("~/.scopeon/launchers"))
        .display()
        .to_string();

    let (support, config_path, summary, notes, docs) = match provider {
        OptimizationProviderId::ClaudeCode => (
            OptimizationSupport::LaunchOnly,
            claude_settings_path().as_deref().map(display_path),
            "Launch profiles tune model choice, effort, permission mode, and cache-friendly session startup.".to_string(),
            vec![
                "Scopeon automates Claude through launch profiles in v1 so it can use official CLI flags without mutating your primary Claude settings file.".to_string(),
                "All Claude presets keep launch-time controls separate from your long-lived user/project settings.".to_string(),
            ],
            vec![
                "https://code.claude.com/docs/en/cli-reference".to_string(),
                "https://code.claude.com/docs/en/settings".to_string(),
                "https://code.claude.com/docs/en/changelog".to_string(),
            ],
        ),
        OptimizationProviderId::CopilotCli => (
            OptimizationSupport::LaunchOnly,
            copilot_config_path().as_deref().map(display_path),
            "Launch profiles tune approvals, tool/path access, and how aggressively Copilot can act without waiting.".to_string(),
            vec![
                "Scopeon keeps Copilot presets launch-only in v1 because the highest-value efficiency controls are official CLI flags and interactive commands rather than a stable preset file.".to_string(),
                "Use the generated launchers together with in-session commands like /model and /compact for finer control.".to_string(),
            ],
            vec![
                "https://docs.github.com/en/copilot/how-tos/copilot-cli/set-up-copilot-cli/configure-copilot-cli".to_string(),
                "https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot".to_string(),
                "https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference".to_string(),
                "https://code.visualstudio.com/updates/v1_118#_improving-token-efficiency".to_string(),
            ],
        ),
        OptimizationProviderId::Codex => (
            OptimizationSupport::ConfigAndLaunch,
            codex_config_path().as_deref().map(display_path),
            "Codex presets map to real config.toml profiles plus matching launchers, so you can switch cost/speed/power without hand-editing TOML.".to_string(),
            vec![
                "Scopeon writes provider-owned profiles into ~/.codex/config.toml and keeps a matching launcher for each preset.".to_string(),
                "The selected preset is also stored in Scopeon's config so the TUI and APIs can show what was last applied.".to_string(),
            ],
            vec![
                "https://developers.openai.com/codex/config-basic".to_string(),
                "https://developers.openai.com/codex/config-advanced".to_string(),
                "https://developers.openai.com/codex/config-reference".to_string(),
                "https://github.com/openai/codex/blob/main/docs/config.md".to_string(),
            ],
        ),
        OptimizationProviderId::GeminiCli => (
            OptimizationSupport::ConfigAndLaunch,
            gemini_settings_path().as_deref().map(display_path),
            "Gemini presets write Scopeon-managed settings overrides and launchers, letting you tune model, compression, summarization, and discovery depth safely.".to_string(),
            vec![
                "Scopeon uses Gemini's documented settings.json structure plus the GEMINI_CLI_SYSTEM_SETTINGS_PATH override to apply presets without clobbering your personal ~/.gemini/settings.json.".to_string(),
                "The generated override files stay under ~/.scopeon so they are easy to inspect or delete.".to_string(),
            ],
            vec![
                "https://google-gemini.github.io/gemini-cli/docs/get-started/configuration.html".to_string(),
                "https://google-gemini.github.io/gemini-cli/docs/cli/enterprise.html".to_string(),
                "https://google-gemini.github.io/gemini-cli/docs/tools/shell.html".to_string(),
            ],
        ),
    };

    let presets = OptimizationPresetId::all()
        .into_iter()
        .map(|preset| preset_descriptor(provider, preset))
        .collect();

    ProviderOptimizationReport {
        provider_id: provider.as_str().to_string(),
        provider_name: provider.display_name().to_string(),
        detected,
        support,
        config_path,
        launcher_dir,
        current_preset,
        summary,
        notes,
        docs,
        presets,
    }
}

pub fn preview_provider_preset(
    provider: OptimizationProviderId,
    preset: OptimizationPresetId,
    _user_config: &UserConfig,
) -> Result<PresetPreview> {
    let descriptor = preset_descriptor(provider, preset);
    let launcher = launcher_artifact(provider, preset)?;
    let mut artifacts = vec![FileArtifactPreview {
        kind: "launcher".to_string(),
        path: display_path(&launcher.path),
        description: launcher.description,
        content: launcher.content,
    }];
    let mut warnings = Vec::new();

    match provider {
        OptimizationProviderId::Codex => {
            artifacts.push(codex_preview_artifact(preset)?);
            warnings.push(
                "Codex preview shows the Scopeon-managed profile block that will be merged into ~/.codex/config.toml; existing unrelated keys are preserved.".to_string(),
            );
        },
        OptimizationProviderId::GeminiCli => {
            artifacts.push(gemini_preview_artifact(preset)?);
            warnings.push(
                "Gemini presets write a Scopeon-managed system-override JSON file and point GEMINI_CLI_SYSTEM_SETTINGS_PATH at it from the launcher.".to_string(),
            );
        },
        OptimizationProviderId::ClaudeCode => {
            warnings.push(
                "Claude presets are launch-only in v1. Scopeon avoids mutating ~/.claude/settings.json and instead generates an explicit launcher with official CLI flags.".to_string(),
            );
        },
        OptimizationProviderId::CopilotCli => {
            warnings.push(
                "Copilot presets are launch-only in v1 because the most reliable efficiency controls are official CLI flags and in-session commands.".to_string(),
            );
        },
    }

    Ok(PresetPreview {
        provider_id: provider.as_str().to_string(),
        provider_name: provider.display_name().to_string(),
        preset_id: preset.as_str().to_string(),
        support: provider_support(provider),
        launcher_path: display_path(&launcher.path),
        launch_command: descriptor.command_preview,
        artifacts,
        warnings,
    })
}

pub fn apply_provider_preset(
    provider: OptimizationProviderId,
    preset: OptimizationPresetId,
    user_config: &mut UserConfig,
) -> Result<ApplyReport> {
    let descriptor = preset_descriptor(provider, preset);
    let launcher = launcher_artifact(provider, preset)?;
    let mut files_written = Vec::new();
    let mut warnings = Vec::new();

    match provider {
        OptimizationProviderId::Codex => {
            let path = codex_config_path()
                .ok_or_else(|| anyhow::anyhow!("Cannot resolve Codex config path"))?;
            apply_codex_profile(&path, preset)?;
            files_written.push(display_path(&path));
        },
        OptimizationProviderId::GeminiCli => {
            let artifact = gemini_override_artifact(preset)?;
            write_atomic_text(&artifact.path, &artifact.content, false)?;
            files_written.push(display_path(&artifact.path));
        },
        OptimizationProviderId::ClaudeCode => {
            warnings.push(
                "Claude apply writes a launcher only. Your primary ~/.claude/settings.json is left untouched.".to_string(),
            );
        },
        OptimizationProviderId::CopilotCli => {
            warnings.push(
                "Copilot apply writes a launcher only. Your primary ~/.copilot/config.json is left untouched.".to_string(),
            );
        },
    }

    write_atomic_text(&launcher.path, &launcher.content, false)?;
    ensure_executable(&launcher.path)?;
    files_written.push(display_path(&launcher.path));

    user_config
        .optimizer
        .applied_presets
        .insert(provider.as_str().to_string(), preset.as_str().to_string());

    Ok(ApplyReport {
        provider_id: provider.as_str().to_string(),
        provider_name: provider.display_name().to_string(),
        preset_id: preset.as_str().to_string(),
        support: provider_support(provider),
        launcher_path: display_path(&launcher.path),
        launch_command: descriptor.command_preview,
        files_written,
        warnings,
    })
}

fn provider_support(provider: OptimizationProviderId) -> OptimizationSupport {
    match provider {
        OptimizationProviderId::ClaudeCode | OptimizationProviderId::CopilotCli => {
            OptimizationSupport::LaunchOnly
        },
        OptimizationProviderId::Codex | OptimizationProviderId::GeminiCli => {
            OptimizationSupport::ConfigAndLaunch
        },
    }
}

fn provider_detected(provider: OptimizationProviderId) -> bool {
    match provider {
        OptimizationProviderId::ClaudeCode => {
            claude_projects_path().map(|p| p.exists()).unwrap_or(false)
                || claude_settings_path().map(|p| p.exists()).unwrap_or(false)
        },
        OptimizationProviderId::CopilotCli => copilot_home()
            .map(|p| {
                p.join("session-state").exists()
                    || p.join("config.json").exists()
                    || p.join("mcp-config.json").exists()
            })
            .unwrap_or(false),
        OptimizationProviderId::Codex => codex_home()
            .map(|p| p.join("sessions").exists() || p.join("config.toml").exists())
            .unwrap_or(false),
        OptimizationProviderId::GeminiCli => gemini_home()
            .map(|p| p.join("tmp").exists() || p.join("settings.json").exists())
            .unwrap_or(false),
    }
}

fn preset_descriptor(
    provider: OptimizationProviderId,
    preset: OptimizationPresetId,
) -> OptimizationPreset {
    match provider {
        OptimizationProviderId::ClaudeCode => {
            let (summary, tradeoff, optimizations, command) = match preset {
                OptimizationPresetId::MostSavings => (
                    "Cheapest Claude flow: Haiku model, low effort, plan mode, and a minimal startup surface.",
                    "Lowest cost and best cache reuse, but less autonomy and less raw model power.",
                    vec![
                        "Uses claude-haiku-4-5 for the lowest-cost default model in Scopeon's price table.".to_string(),
                        "Sets --effort low to reduce extra reasoning tokens.".to_string(),
                        "Starts in plan mode and bare mode to keep tool/context overhead down.".to_string(),
                        "Uses --exclude-dynamic-system-prompt-sections for more cache-stable prompts.".to_string(),
                    ],
                    vec![
                        "claude".to_string(),
                        "--model".to_string(),
                        "claude-haiku-4-5".to_string(),
                        "--effort".to_string(),
                        "low".to_string(),
                        "--permission-mode".to_string(),
                        "plan".to_string(),
                        "--bare".to_string(),
                        "--exclude-dynamic-system-prompt-sections".to_string(),
                    ],
                ),
                OptimizationPresetId::Balanced => (
                    "Recommended default: Sonnet, medium effort, standard permissions, and cache-friendly startup.",
                    "Good quality/cost balance with predictable approvals.",
                    vec![
                        "Uses claude-sonnet-4-5 as the balanced default.".to_string(),
                        "Keeps medium effort for a reasonable reasoning budget.".to_string(),
                        "Preserves the standard permission flow instead of forcing auto mode.".to_string(),
                        "Uses --exclude-dynamic-system-prompt-sections to improve cache reuse.".to_string(),
                    ],
                    vec![
                        "claude".to_string(),
                        "--model".to_string(),
                        "claude-sonnet-4-5".to_string(),
                        "--effort".to_string(),
                        "medium".to_string(),
                        "--permission-mode".to_string(),
                        "default".to_string(),
                        "--exclude-dynamic-system-prompt-sections".to_string(),
                    ],
                ),
                OptimizationPresetId::MostSpeed => (
                    "Fast Claude flow: Sonnet, low effort, auto mode, and bare startup.",
                    "Minimizes waiting and prompt weight, but trusts Claude with more autonomy.",
                    vec![
                        "Uses claude-sonnet-4-5 to keep quality reasonable while staying fast.".to_string(),
                        "Sets --effort low to reduce thinking latency.".to_string(),
                        "Starts in auto mode to avoid repeated approval stops.".to_string(),
                        "Uses --bare plus cache-stable startup flags for a lighter session.".to_string(),
                    ],
                    vec![
                        "claude".to_string(),
                        "--model".to_string(),
                        "claude-sonnet-4-5".to_string(),
                        "--effort".to_string(),
                        "low".to_string(),
                        "--permission-mode".to_string(),
                        "auto".to_string(),
                        "--bare".to_string(),
                        "--exclude-dynamic-system-prompt-sections".to_string(),
                    ],
                ),
                OptimizationPresetId::MostPower => (
                    "Maximum-capability Claude flow: Opus, high effort, auto mode, and full feature discovery.",
                    "Best answer quality and deeper reasoning, but highest spend and tool churn.",
                    vec![
                        "Uses claude-opus-4-7 as the high-power default.".to_string(),
                        "Sets --effort high to bias toward deeper reasoning.".to_string(),
                        "Starts in auto mode so the higher-end model is not bottlenecked by approvals.".to_string(),
                        "Keeps cache-stable prompt sections enabled even in the most powerful profile.".to_string(),
                    ],
                    vec![
                        "claude".to_string(),
                        "--model".to_string(),
                        "claude-opus-4-7".to_string(),
                        "--effort".to_string(),
                        "high".to_string(),
                        "--permission-mode".to_string(),
                        "auto".to_string(),
                        "--exclude-dynamic-system-prompt-sections".to_string(),
                    ],
                ),
            };
            OptimizationPreset {
                id: preset.as_str().to_string(),
                title: preset.title().to_string(),
                summary: summary.to_string(),
                tradeoff: tradeoff.to_string(),
                config_strategy: "Launcher only (official Claude CLI flags).".to_string(),
                command_preview: join_command_preview(&command),
                optimizations,
            }
        },
        OptimizationProviderId::CopilotCli => {
            let (summary, tradeoff, optimizations, command) = match preset {
                OptimizationPresetId::MostSavings => (
                    "Conservative Copilot flow with shell access disabled and temp-dir access removed.",
                    "Reduces tool churn and risk, but may require more manual intervention for build/test tasks.",
                    vec![
                        "Uses --deny-tool='shell' to prevent expensive command loops by default.".to_string(),
                        "Uses --disallow-temp-dir to tighten writable scope.".to_string(),
                        "Pairs well with plan mode and manual /compact usage for long sessions.".to_string(),
                    ],
                    vec![
                        "copilot".to_string(),
                        "--deny-tool=shell".to_string(),
                        "--disallow-temp-dir".to_string(),
                    ],
                ),
                OptimizationPresetId::Balanced => (
                    "Default Copilot flow with no extra permissions beyond the standard prompts.",
                    "Keeps the stock safety model and lets you decide tool approvals interactively.",
                    vec![
                        "Leaves the standard approval model intact.".to_string(),
                        "Best starting point when you want to mix safety with flexibility.".to_string(),
                        "Use /model and /compact inside the session for finer tuning.".to_string(),
                    ],
                    vec!["copilot".to_string()],
                ),
                OptimizationPresetId::MostSpeed => (
                    "Fast Copilot flow that pre-approves tools and paths so the agent can move without pauses.",
                    "Removes a lot of interaction overhead, but increases the blast radius of mistakes.",
                    vec![
                        "Uses --allow-all-tools so tool invocations do not stop for approval.".to_string(),
                        "Uses --allow-all-paths to avoid path-verification pauses.".to_string(),
                        "Still leaves URL verification in place so network access remains explicit.".to_string(),
                    ],
                    vec![
                        "copilot".to_string(),
                        "--allow-all-tools".to_string(),
                        "--allow-all-paths".to_string(),
                    ],
                ),
                OptimizationPresetId::MostPower => (
                    "Maximum-autonomy Copilot flow using the full official --allow-all / --yolo posture.",
                    "Highest throughput and least friction, but also the highest operational risk.",
                    vec![
                        "Uses --allow-all to disable tool, path, and URL verification together.".to_string(),
                        "Best reserved for trusted repos and highly constrained environments.".to_string(),
                        "Combine with /model inside the session if you need a stronger model after launch.".to_string(),
                    ],
                    vec!["copilot".to_string(), "--allow-all".to_string()],
                ),
            };
            OptimizationPreset {
                id: preset.as_str().to_string(),
                title: preset.title().to_string(),
                summary: summary.to_string(),
                tradeoff: tradeoff.to_string(),
                config_strategy: "Launcher only (official Copilot CLI flags).".to_string(),
                command_preview: join_command_preview(&command),
                optimizations,
            }
        },
        OptimizationProviderId::Codex => {
            let (summary, tradeoff, profile_values) = codex_profile_values(preset);
            let command = vec![
                "codex".to_string(),
                "--profile".to_string(),
                codex_profile_name(preset),
            ];
            OptimizationPreset {
                id: preset.as_str().to_string(),
                title: preset.title().to_string(),
                summary: summary.to_string(),
                tradeoff: tradeoff.to_string(),
                config_strategy: "Adds or updates a Scopeon profile in ~/.codex/config.toml and writes a matching launcher.".to_string(),
                command_preview: join_command_preview(&command),
                optimizations: profile_values,
            }
        },
        OptimizationProviderId::GeminiCli => {
            let (summary, tradeoff, override_content) = gemini_override_values(preset);
            let command = vec!["gemini".to_string()];
            OptimizationPreset {
                id: preset.as_str().to_string(),
                title: preset.title().to_string(),
                summary: summary.to_string(),
                tradeoff: tradeoff.to_string(),
                config_strategy: "Writes a Scopeon-managed Gemini settings override file and a launcher that points GEMINI_CLI_SYSTEM_SETTINGS_PATH at it.".to_string(),
                command_preview: join_command_preview(&command),
                optimizations: override_content,
            }
        },
    }
}

fn codex_profile_values(preset: OptimizationPresetId) -> (String, String, Vec<String>) {
    match preset {
        OptimizationPresetId::MostSavings => (
            "Lower-cost Codex profile with a smaller model, low reasoning effort, cached web search, and interactive approvals.".to_string(),
            "Best cost control, but slower throughput than fully autonomous presets.".to_string(),
            vec![
                "model = gpt-5.4-mini".to_string(),
                "approval_policy = on-request".to_string(),
                "default_permissions = :workspace".to_string(),
                "web_search = cached".to_string(),
                "model_reasoning_effort = low".to_string(),
            ],
        ),
        OptimizationPresetId::Balanced => (
            "Balanced Codex profile with GPT-5.4, medium reasoning, workspace permissions, and cached web search.".to_string(),
            "Good default when you want stronger answers without paying the full autonomy or live-search premium.".to_string(),
            vec![
                "model = gpt-5.4".to_string(),
                "approval_policy = on-request".to_string(),
                "default_permissions = :workspace".to_string(),
                "web_search = cached".to_string(),
                "model_reasoning_effort = medium".to_string(),
            ],
        ),
        OptimizationPresetId::MostSpeed => (
            "Fast Codex profile with GPT-5.4-mini, minimal reasoning, fast tier, and non-interactive approvals.".to_string(),
            "Minimizes latency and interruptions, but assumes you trust the repo and the agent path.".to_string(),
            vec![
                "model = gpt-5.4-mini".to_string(),
                "approval_policy = never".to_string(),
                "default_permissions = :workspace".to_string(),
                "web_search = cached".to_string(),
                "model_reasoning_effort = minimal".to_string(),
                "service_tier = fast".to_string(),
            ],
        ),
        OptimizationPresetId::MostPower => (
            "High-power Codex profile with GPT-5.4, high reasoning effort, live web search, and non-interactive approvals.".to_string(),
            "Best throughput and strongest reasoning, but the highest spend and least human friction.".to_string(),
            vec![
                "model = gpt-5.4".to_string(),
                "approval_policy = never".to_string(),
                "default_permissions = :workspace".to_string(),
                "web_search = live".to_string(),
                "model_reasoning_effort = high".to_string(),
            ],
        ),
    }
}

fn gemini_override_values(preset: OptimizationPresetId) -> (String, String, Vec<String>) {
    match preset {
        OptimizationPresetId::MostSavings => (
            "Lean Gemini profile with Flash, aggressive compression, small tool-output summaries, and shallow discovery.".to_string(),
            "Cheapest and safest, but trims history and context discovery earlier.".to_string(),
            vec![
                "model.name = gemini-2.5-flash".to_string(),
                "model.maxSessionTurns = 12".to_string(),
                "model.chatCompression.contextPercentageThreshold = 0.55".to_string(),
                "model.summarizeToolOutput.run_shell_command.tokenBudget = 600".to_string(),
                "context.discoveryMaxDirs = 80".to_string(),
                "context.fileFiltering.enableRecursiveFileSearch = false".to_string(),
            ],
        ),
        OptimizationPresetId::Balanced => (
            "Balanced Gemini profile with Pro, moderate compression, and bounded tool-output summaries.".to_string(),
            "Keeps quality high without leaving every session effectively unbounded.".to_string(),
            vec![
                "model.name = gemini-2.5-pro".to_string(),
                "model.maxSessionTurns = 20".to_string(),
                "model.chatCompression.contextPercentageThreshold = 0.65".to_string(),
                "model.summarizeToolOutput.run_shell_command.tokenBudget = 1200".to_string(),
                "context.discoveryMaxDirs = 120".to_string(),
                "context.fileFiltering.enableRecursiveFileSearch = true".to_string(),
            ],
        ),
        OptimizationPresetId::MostSpeed => (
            "Fast Gemini profile with Flash, earlier compression, and very small shell-output summaries.".to_string(),
            "Optimized for quick loops and lighter context, but may omit detail from tool-heavy turns.".to_string(),
            vec![
                "model.name = gemini-2.5-flash".to_string(),
                "model.maxSessionTurns = 16".to_string(),
                "model.chatCompression.contextPercentageThreshold = 0.50".to_string(),
                "model.summarizeToolOutput.run_shell_command.tokenBudget = 400".to_string(),
                "context.discoveryMaxDirs = 60".to_string(),
                "context.fileFiltering.enableRecursiveFileSearch = false".to_string(),
            ],
        ),
        OptimizationPresetId::MostPower => (
            "High-power Gemini profile with Pro, late compression, larger tool summaries, and broad discovery.".to_string(),
            "Best recall and context depth, but also the highest token pressure.".to_string(),
            vec![
                "model.name = gemini-2.5-pro".to_string(),
                "model.maxSessionTurns = -1".to_string(),
                "model.chatCompression.contextPercentageThreshold = 0.80".to_string(),
                "model.summarizeToolOutput.run_shell_command.tokenBudget = 2000".to_string(),
                "context.discoveryMaxDirs = 200".to_string(),
                "context.fileFiltering.enableRecursiveFileSearch = true".to_string(),
            ],
        ),
    }
}

fn claude_projects_path() -> Option<PathBuf> {
    claude_home().map(|base| base.join("projects"))
}

fn claude_settings_path() -> Option<PathBuf> {
    claude_home().map(|base| base.join("settings.json"))
}

fn claude_home() -> Option<PathBuf> {
    std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))
}

fn copilot_home() -> Option<PathBuf> {
    std::env::var("COPILOT_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".copilot")))
}

fn copilot_config_path() -> Option<PathBuf> {
    copilot_home().map(|home| home.join("config.json"))
}

fn codex_home() -> Option<PathBuf> {
    std::env::var("CODEX_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::var("CODEX_CONFIG_DIR").ok().map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
}

fn codex_config_path() -> Option<PathBuf> {
    codex_home().map(|home| home.join("config.toml"))
}

fn gemini_home() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".gemini"))
}

fn gemini_settings_path() -> Option<PathBuf> {
    gemini_home().map(|home| home.join("settings.json"))
}

fn launcher_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
    Ok(home.join(".scopeon").join("launchers"))
}

fn optimizer_asset_dir(provider: OptimizationProviderId) -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot find home directory"))?;
    Ok(home
        .join(".scopeon")
        .join("optimizer")
        .join(provider.as_str()))
}

#[derive(Debug, Clone)]
struct LauncherArtifact {
    path: PathBuf,
    description: String,
    content: String,
}

fn launcher_artifact(
    provider: OptimizationProviderId,
    preset: OptimizationPresetId,
) -> Result<LauncherArtifact> {
    let path = launcher_dir()?.join(launcher_file_name(provider, preset));
    let command = launch_command_parts(provider, preset);
    let mut env = HashMap::<String, String>::new();
    if provider == OptimizationProviderId::GeminiCli {
        let override_path = gemini_override_path(preset)?;
        env.insert(
            "GEMINI_CLI_SYSTEM_SETTINGS_PATH".to_string(),
            override_path.display().to_string(),
        );
    }
    let content = render_launcher_script(&env, &command);
    let description = format!(
        "Launch {} with the '{}' Scopeon preset",
        provider.display_name(),
        preset.as_str()
    );
    Ok(LauncherArtifact {
        path,
        description,
        content,
    })
}

fn launcher_file_name(provider: OptimizationProviderId, preset: OptimizationPresetId) -> String {
    if cfg!(windows) {
        format!("{}-{}.cmd", provider.as_str(), preset.as_str())
    } else {
        format!("{}-{}", provider.as_str(), preset.as_str())
    }
}

fn launch_command_parts(
    provider: OptimizationProviderId,
    preset: OptimizationPresetId,
) -> Vec<String> {
    match provider {
        OptimizationProviderId::ClaudeCode => match preset {
            OptimizationPresetId::MostSavings => vec![
                "claude".to_string(),
                "--model".to_string(),
                "claude-haiku-4-5".to_string(),
                "--effort".to_string(),
                "low".to_string(),
                "--permission-mode".to_string(),
                "plan".to_string(),
                "--bare".to_string(),
                "--exclude-dynamic-system-prompt-sections".to_string(),
            ],
            OptimizationPresetId::Balanced => vec![
                "claude".to_string(),
                "--model".to_string(),
                "claude-sonnet-4-5".to_string(),
                "--effort".to_string(),
                "medium".to_string(),
                "--permission-mode".to_string(),
                "default".to_string(),
                "--exclude-dynamic-system-prompt-sections".to_string(),
            ],
            OptimizationPresetId::MostSpeed => vec![
                "claude".to_string(),
                "--model".to_string(),
                "claude-sonnet-4-5".to_string(),
                "--effort".to_string(),
                "low".to_string(),
                "--permission-mode".to_string(),
                "auto".to_string(),
                "--bare".to_string(),
                "--exclude-dynamic-system-prompt-sections".to_string(),
            ],
            OptimizationPresetId::MostPower => vec![
                "claude".to_string(),
                "--model".to_string(),
                "claude-opus-4-7".to_string(),
                "--effort".to_string(),
                "high".to_string(),
                "--permission-mode".to_string(),
                "auto".to_string(),
                "--exclude-dynamic-system-prompt-sections".to_string(),
            ],
        },
        OptimizationProviderId::CopilotCli => match preset {
            OptimizationPresetId::MostSavings => vec![
                "copilot".to_string(),
                "--deny-tool=shell".to_string(),
                "--disallow-temp-dir".to_string(),
            ],
            OptimizationPresetId::Balanced => vec!["copilot".to_string()],
            OptimizationPresetId::MostSpeed => vec![
                "copilot".to_string(),
                "--allow-all-tools".to_string(),
                "--allow-all-paths".to_string(),
            ],
            OptimizationPresetId::MostPower => {
                vec!["copilot".to_string(), "--allow-all".to_string()]
            },
        },
        OptimizationProviderId::Codex => vec![
            "codex".to_string(),
            "--profile".to_string(),
            codex_profile_name(preset),
        ],
        OptimizationProviderId::GeminiCli => vec!["gemini".to_string()],
    }
}

fn codex_profile_name(preset: OptimizationPresetId) -> String {
    format!("scopeon-{}", preset.as_str())
}

fn codex_preview_artifact(preset: OptimizationPresetId) -> Result<FileArtifactPreview> {
    let config_path =
        codex_config_path().ok_or_else(|| anyhow::anyhow!("Cannot resolve Codex config path"))?;
    let profile_name = codex_profile_name(preset);
    let mut snippet = String::new();
    snippet.push_str(&format!("profile = \"{}\"\n\n", profile_name));
    snippet.push_str(&format!("[profiles.{}]\n", profile_name));
    for (key, value) in codex_profile_pairs(preset) {
        snippet.push_str(&format!("{key} = {value}\n"));
    }
    Ok(FileArtifactPreview {
        kind: "config".to_string(),
        path: display_path(&config_path),
        description:
            "Scopeon-managed profile block to merge into ~/.codex/config.toml and activate as the default profile.".to_string(),
        content: snippet,
    })
}

fn apply_codex_profile(path: &Path, preset: OptimizationPresetId) -> Result<()> {
    let mut root = load_toml_table(path)?;
    let profile_name = codex_profile_name(preset);
    root.insert(
        "profile".to_string(),
        toml::Value::String(profile_name.clone()),
    );

    let profiles = ensure_toml_table(&mut root, "profiles");
    let profile = ensure_toml_table(profiles, &profile_name);
    for (key, value) in codex_profile_pairs(preset) {
        profile.insert(key.to_string(), value);
    }

    let rendered = toml::to_string_pretty(&root).context("Failed to render Codex config TOML")?;
    write_atomic_text(path, &rendered, true)
}

fn codex_profile_pairs(preset: OptimizationPresetId) -> Vec<(&'static str, toml::Value)> {
    match preset {
        OptimizationPresetId::MostSavings => vec![
            ("model", toml::Value::String("gpt-5.4-mini".to_string())),
            (
                "approval_policy",
                toml::Value::String("on-request".to_string()),
            ),
            (
                "default_permissions",
                toml::Value::String(":workspace".to_string()),
            ),
            ("web_search", toml::Value::String("cached".to_string())),
            (
                "model_reasoning_effort",
                toml::Value::String("low".to_string()),
            ),
        ],
        OptimizationPresetId::Balanced => vec![
            ("model", toml::Value::String("gpt-5.4".to_string())),
            (
                "approval_policy",
                toml::Value::String("on-request".to_string()),
            ),
            (
                "default_permissions",
                toml::Value::String(":workspace".to_string()),
            ),
            ("web_search", toml::Value::String("cached".to_string())),
            (
                "model_reasoning_effort",
                toml::Value::String("medium".to_string()),
            ),
        ],
        OptimizationPresetId::MostSpeed => vec![
            ("model", toml::Value::String("gpt-5.4-mini".to_string())),
            ("approval_policy", toml::Value::String("never".to_string())),
            (
                "default_permissions",
                toml::Value::String(":workspace".to_string()),
            ),
            ("web_search", toml::Value::String("cached".to_string())),
            (
                "model_reasoning_effort",
                toml::Value::String("minimal".to_string()),
            ),
            ("service_tier", toml::Value::String("fast".to_string())),
        ],
        OptimizationPresetId::MostPower => vec![
            ("model", toml::Value::String("gpt-5.4".to_string())),
            ("approval_policy", toml::Value::String("never".to_string())),
            (
                "default_permissions",
                toml::Value::String(":workspace".to_string()),
            ),
            ("web_search", toml::Value::String("live".to_string())),
            (
                "model_reasoning_effort",
                toml::Value::String("high".to_string()),
            ),
        ],
    }
}

fn gemini_override_path(preset: OptimizationPresetId) -> Result<PathBuf> {
    Ok(optimizer_asset_dir(OptimizationProviderId::GeminiCli)?
        .join(format!("{}.settings.json", preset.as_str())))
}

fn gemini_preview_artifact(preset: OptimizationPresetId) -> Result<FileArtifactPreview> {
    let artifact = gemini_override_artifact(preset)?;
    Ok(FileArtifactPreview {
        kind: "config".to_string(),
        path: display_path(&artifact.path),
        description:
            "Scopeon-managed Gemini system override file. The generated launcher sets GEMINI_CLI_SYSTEM_SETTINGS_PATH to this file.".to_string(),
        content: artifact.content,
    })
}

fn gemini_override_artifact(preset: OptimizationPresetId) -> Result<LauncherArtifact> {
    let path = gemini_override_path(preset)?;
    let content = serde_json::to_string_pretty(&gemini_override_json(preset))
        .context("Failed to render Gemini override JSON")?;
    Ok(LauncherArtifact {
        path,
        description: format!("Gemini settings override for {}", preset.as_str()),
        content,
    })
}

fn gemini_override_json(preset: OptimizationPresetId) -> serde_json::Value {
    match preset {
        OptimizationPresetId::MostSavings => serde_json::json!({
            "model": {
                "name": "gemini-2.5-flash",
                "maxSessionTurns": 12,
                "chatCompression": { "contextPercentageThreshold": 0.55 },
                "summarizeToolOutput": {
                    "run_shell_command": { "tokenBudget": 600 }
                }
            },
            "context": {
                "discoveryMaxDirs": 80,
                "fileFiltering": {
                    "respectGitIgnore": true,
                    "respectGeminiIgnore": true,
                    "enableRecursiveFileSearch": false
                }
            }
        }),
        OptimizationPresetId::Balanced => serde_json::json!({
            "model": {
                "name": "gemini-2.5-pro",
                "maxSessionTurns": 20,
                "chatCompression": { "contextPercentageThreshold": 0.65 },
                "summarizeToolOutput": {
                    "run_shell_command": { "tokenBudget": 1200 }
                }
            },
            "context": {
                "discoveryMaxDirs": 120,
                "fileFiltering": {
                    "respectGitIgnore": true,
                    "respectGeminiIgnore": true,
                    "enableRecursiveFileSearch": true
                }
            }
        }),
        OptimizationPresetId::MostSpeed => serde_json::json!({
            "model": {
                "name": "gemini-2.5-flash",
                "maxSessionTurns": 16,
                "chatCompression": { "contextPercentageThreshold": 0.50 },
                "summarizeToolOutput": {
                    "run_shell_command": { "tokenBudget": 400 }
                }
            },
            "context": {
                "discoveryMaxDirs": 60,
                "fileFiltering": {
                    "respectGitIgnore": true,
                    "respectGeminiIgnore": true,
                    "enableRecursiveFileSearch": false
                }
            }
        }),
        OptimizationPresetId::MostPower => serde_json::json!({
            "model": {
                "name": "gemini-2.5-pro",
                "maxSessionTurns": -1,
                "chatCompression": { "contextPercentageThreshold": 0.80 },
                "summarizeToolOutput": {
                    "run_shell_command": { "tokenBudget": 2000 }
                }
            },
            "context": {
                "discoveryMaxDirs": 200,
                "fileFiltering": {
                    "respectGitIgnore": true,
                    "respectGeminiIgnore": true,
                    "enableRecursiveFileSearch": true
                }
            }
        }),
    }
}

fn render_launcher_script(env: &HashMap<String, String>, command: &[String]) -> String {
    if cfg!(windows) {
        let mut lines = vec!["@echo off".to_string(), "setlocal".to_string()];
        for (key, value) in env {
            lines.push(format!("set \"{}={}\"", key, value.replace('"', "\"\"")));
        }
        let cmd = command
            .iter()
            .map(|part| windows_quote(part))
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!("{cmd} %*"));
        lines.push("endlocal".to_string());
        return lines.join("\r\n");
    }

    let mut lines = vec![
        "#!/usr/bin/env bash".to_string(),
        "set -euo pipefail".to_string(),
    ];
    for (key, value) in env {
        lines.push(format!("export {}={}", key, shell_quote(value)));
    }
    let cmd = command
        .iter()
        .map(|part| shell_quote(part))
        .collect::<Vec<_>>()
        .join(" ");
    lines.push(format!("exec {} \"$@\"", cmd));
    lines.join("\n")
}

fn load_toml_table(path: &Path) -> Result<Table> {
    if !path.exists() {
        return Ok(Table::new());
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    let value = toml::from_str::<toml::Value>(&raw)
        .with_context(|| format!("Parsing TOML at {}", path.display()))?;
    match value {
        toml::Value::Table(table) => Ok(table),
        _ => anyhow::bail!("Expected a TOML table at {}", path.display()),
    }
}

fn ensure_toml_table<'a>(table: &'a mut Table, key: &str) -> &'a mut Table {
    if !matches!(table.get(key), Some(toml::Value::Table(_))) {
        table.insert(key.to_string(), toml::Value::Table(Table::new()));
    }
    match table.get_mut(key) {
        Some(toml::Value::Table(inner)) => inner,
        _ => unreachable!(),
    }
}

fn write_atomic_text(path: &Path, content: &str, backup_existing: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    if backup_existing && path.exists() {
        let backup = path.with_extension(match path.extension().and_then(|e| e.to_str()) {
            Some(ext) if !ext.is_empty() => format!("{ext}.bak"),
            _ => "bak".to_string(),
        });
        let _ = fs::copy(path, backup);
    }
    let tmp = path.with_extension(match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() => format!("{ext}.tmp"),
        _ => "tmp".to_string(),
    });
    fs::write(&tmp, content).with_context(|| format!("Failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("Failed to replace {}", path.display()))?;
    Ok(())
}

fn ensure_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

fn display_path(path: &Path) -> String {
    path.display().to_string()
}

fn join_command_preview(parts: &[String]) -> String {
    if cfg!(windows) {
        parts
            .iter()
            .map(|part| windows_quote(part))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        parts
            .iter()
            .map(|part| shell_quote(part))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.' | ':' | '='))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

fn windows_quote(value: &str) -> String {
    if value.contains([' ', '"']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn provider_aliases_cover_short_names() {
        assert_eq!(
            OptimizationProviderId::from_alias("gemini"),
            Some(OptimizationProviderId::GeminiCli)
        );
        assert_eq!(
            OptimizationProviderId::from_alias("copilot"),
            Some(OptimizationProviderId::CopilotCli)
        );
    }

    #[test]
    fn preset_aliases_cover_short_names() {
        assert_eq!(
            OptimizationPresetId::from_alias("speed"),
            Some(OptimizationPresetId::MostSpeed)
        );
        assert_eq!(
            OptimizationPresetId::from_alias("power"),
            Some(OptimizationPresetId::MostPower)
        );
    }

    #[test]
    fn codex_preview_contains_scopeon_profile() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("CODEX_HOME", dir.path()) };
        let preview = codex_preview_artifact(OptimizationPresetId::MostSavings).unwrap();
        assert!(preview.content.contains("[profiles.scopeon-most-savings]"));
        unsafe { std::env::remove_var("CODEX_HOME") };
    }

    #[test]
    fn gemini_preview_uses_settings_override_file() {
        let preview =
            gemini_preview_artifact(OptimizationPresetId::Balanced).expect("gemini preview");
        assert!(preview.path.ends_with("balanced.settings.json"));
        assert!(preview.content.contains("\"gemini-2.5-pro\""));
    }
}
