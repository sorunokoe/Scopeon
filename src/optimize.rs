use anyhow::{anyhow, Result};
use clap::{Subcommand, ValueEnum};

use scopeon_core::{
    apply_provider_preset, list_provider_optimization_reports, preview_provider_preset,
    OptimizationPresetId, OptimizationProviderId, UserConfig,
};

#[derive(Debug, Clone, Subcommand)]
pub enum OptimizeAction {
    /// Scan supported providers and show what Scopeon can optimize.
    Scan,
    /// Explain the available presets and controls for one provider or all supported providers.
    Explain {
        #[arg(long, value_enum)]
        provider: Option<ProviderArg>,
    },
    /// Preview the files and launchers Scopeon would generate for a preset.
    Preview {
        #[arg(long, value_enum)]
        provider: Option<ProviderArg>,
        #[arg(long, value_enum)]
        preset: PresetArg,
    },
    /// Apply a preset by writing launchers and any supported config artifacts.
    Apply {
        #[arg(long, value_enum)]
        provider: Option<ProviderArg>,
        #[arg(long, value_enum)]
        preset: PresetArg,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ProviderArg {
    #[value(name = "claude-code")]
    ClaudeCode,
    #[value(name = "copilot-cli")]
    CopilotCli,
    #[value(name = "codex")]
    Codex,
    #[value(name = "gemini-cli")]
    GeminiCli,
}

impl From<ProviderArg> for OptimizationProviderId {
    fn from(value: ProviderArg) -> Self {
        match value {
            ProviderArg::ClaudeCode => OptimizationProviderId::ClaudeCode,
            ProviderArg::CopilotCli => OptimizationProviderId::CopilotCli,
            ProviderArg::Codex => OptimizationProviderId::Codex,
            ProviderArg::GeminiCli => OptimizationProviderId::GeminiCli,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PresetArg {
    #[value(name = "most-savings")]
    MostSavings,
    #[value(name = "balanced")]
    Balanced,
    #[value(name = "most-speed")]
    MostSpeed,
    #[value(name = "most-power")]
    MostPower,
}

impl From<PresetArg> for OptimizationPresetId {
    fn from(value: PresetArg) -> Self {
        match value {
            PresetArg::MostSavings => OptimizationPresetId::MostSavings,
            PresetArg::Balanced => OptimizationPresetId::Balanced,
            PresetArg::MostSpeed => OptimizationPresetId::MostSpeed,
            PresetArg::MostPower => OptimizationPresetId::MostPower,
        }
    }
}

pub fn cmd_optimize(action: OptimizeAction, user_config: &mut UserConfig) -> Result<()> {
    match action {
        OptimizeAction::Scan => cmd_scan(user_config),
        OptimizeAction::Explain { provider } => cmd_explain(provider, user_config),
        OptimizeAction::Preview { provider, preset } => {
            cmd_preview(provider, preset.into(), user_config)
        },
        OptimizeAction::Apply { provider, preset } => {
            cmd_apply(provider, preset.into(), user_config)
        },
    }
}

fn cmd_scan(user_config: &UserConfig) -> Result<()> {
    let reports = list_provider_optimization_reports(user_config);
    println!(
        "{:<18} {:<9} {:<17} {:<15} Config path",
        "Provider", "Detected", "Support", "Current preset"
    );
    println!("{}", "-".repeat(96));
    for report in &reports {
        let detected = if report.detected { "yes" } else { "no" };
        let preset = report.current_preset.as_deref().unwrap_or("none");
        let config = report.config_path.as_deref().unwrap_or("n/a");
        println!(
            "{:<18} {:<9} {:<17} {:<15} {}",
            report.provider_name,
            detected,
            report.support.label(),
            preset,
            config
        );
    }
    println!();
    println!("Use `scopeon optimize explain --provider <id>` to inspect a provider.");
    println!("Use `scopeon optimize preview --provider <id> --preset <mode>` before applying.");
    Ok(())
}

fn cmd_explain(provider: Option<ProviderArg>, user_config: &UserConfig) -> Result<()> {
    let reports = selected_reports(provider, user_config)?;
    for (idx, report) in reports.iter().enumerate() {
        if idx > 0 {
            println!();
            println!("{}", "=".repeat(88));
            println!();
        }
        println!("{} ({})", report.provider_name, report.provider_id);
        println!("  Support:      {}", report.support.label());
        println!(
            "  Detected:     {}",
            if report.detected { "yes" } else { "no" }
        );
        println!(
            "  Config path:  {}",
            report.config_path.as_deref().unwrap_or("n/a")
        );
        println!("  Launcher dir: {}", report.launcher_dir);
        if let Some(current) = &report.current_preset {
            println!("  Current:      {}", current);
        }
        println!();
        println!("{}", report.summary);
        if !report.notes.is_empty() {
            println!();
            println!("Notes:");
            for note in &report.notes {
                println!("  - {}", note);
            }
        }
        println!();
        println!("Presets:");
        for preset in &report.presets {
            println!("  {}:", preset.title);
            println!("    {}", preset.summary);
            println!("    Trade-off: {}", preset.tradeoff);
            println!("    Strategy:  {}", preset.config_strategy);
            println!("    Command:   {}", preset.command_preview);
            for item in &preset.optimizations {
                println!("    - {}", item);
            }
        }
        println!();
        println!("Docs:");
        for doc in &report.docs {
            println!("  - {}", doc);
        }
    }
    Ok(())
}

fn cmd_preview(
    provider: Option<ProviderArg>,
    preset: OptimizationPresetId,
    user_config: &UserConfig,
) -> Result<()> {
    let targets = selected_providers(provider, user_config)?;
    for (idx, provider) in targets.iter().enumerate() {
        if idx > 0 {
            println!();
            println!("{}", "=".repeat(88));
            println!();
        }
        let preview = preview_provider_preset(*provider, preset, user_config)?;
        println!("{} — {}", preview.provider_name, preview.preset_id);
        println!("  Support:      {}", preview.support.label());
        println!("  Launcher:     {}", preview.launcher_path);
        println!("  Launch cmd:   {}", preview.launch_command);
        if !preview.warnings.is_empty() {
            println!();
            println!("Warnings:");
            for warning in &preview.warnings {
                println!("  - {}", warning);
            }
        }
        println!();
        for artifact in &preview.artifacts {
            println!("{}: {}", artifact.kind, artifact.path);
            println!("  {}", artifact.description);
            println!();
            println!("{}", artifact.content);
            println!();
        }
    }
    Ok(())
}

fn cmd_apply(
    provider: Option<ProviderArg>,
    preset: OptimizationPresetId,
    user_config: &mut UserConfig,
) -> Result<()> {
    let targets = selected_providers(provider, user_config)?;
    let mut reports = Vec::new();
    for provider in &targets {
        let report = apply_provider_preset(*provider, preset, user_config)?;
        reports.push(report);
    }
    user_config.save()?;

    for (idx, report) in reports.iter().enumerate() {
        if idx > 0 {
            println!();
        }
        println!(
            "Applied {} preset to {}",
            report.preset_id, report.provider_name
        );
        println!("  Support:    {}", report.support.label());
        println!("  Launcher:   {}", report.launcher_path);
        println!("  Launch cmd: {}", report.launch_command);
        println!("  Files:");
        for file in &report.files_written {
            println!("    - {}", file);
        }
        for warning in &report.warnings {
            println!("  Warning: {}", warning);
        }
    }
    println!();
    println!("Recorded applied presets in ~/.scopeon/config.toml under [optimizer].");
    Ok(())
}

fn selected_reports(
    provider: Option<ProviderArg>,
    user_config: &UserConfig,
) -> Result<Vec<scopeon_core::ProviderOptimizationReport>> {
    let reports = list_provider_optimization_reports(user_config);
    if let Some(provider) = provider {
        let wanted = OptimizationProviderId::from(provider).as_str();
        let report = reports
            .into_iter()
            .find(|report| report.provider_id == wanted)
            .ok_or_else(|| anyhow!("Unknown provider '{}'", wanted))?;
        Ok(vec![report])
    } else {
        Ok(reports)
    }
}

fn selected_providers(
    provider: Option<ProviderArg>,
    user_config: &UserConfig,
) -> Result<Vec<OptimizationProviderId>> {
    if let Some(provider) = provider {
        return Ok(vec![provider.into()]);
    }
    let reports = list_provider_optimization_reports(user_config);
    let mut detected: Vec<_> = reports
        .into_iter()
        .filter(|report| report.detected)
        .map(|report| {
            OptimizationProviderId::from_alias(&report.provider_id)
                .expect("provider ids from core reports are always parseable")
        })
        .collect();
    if detected.is_empty() {
        detected = OptimizationProviderId::all().into_iter().collect();
    }
    Ok(detected)
}
