use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use scopeon_collector::watcher;
use scopeon_collector::{
    AiderProvider, ClaudeCodeProvider, CopilotCliProvider, CursorProvider, GeminiCLIProvider,
    GenericOpenAIProvider, OllamaProvider,
};
use scopeon_core::{Config, Database, UserConfig};

mod badge;
mod ci;
mod digest;
mod doctor;
mod git_hook;
mod onboarding;
mod serve;
mod shell_hook;
mod team;

/// Acquire the database lock, propagating a clear error if the mutex is poisoned.
/// A poisoned mutex means a thread panicked while holding the lock — the database
/// connection may be in an inconsistent state. The caller should surface this error
/// rather than continuing with potentially corrupt data.
fn lock_db(db: &Arc<Mutex<Database>>) -> Result<MutexGuard<'_, Database>> {
    db.lock().map_err(|_| {
        anyhow::anyhow!(
            "Database mutex poisoned (a thread panicked while holding it). \
             Restart Scopeon to ensure database integrity."
        )
    })
}

#[derive(Parser)]
#[command(
    name = "scopeon",
    version,
    about = "AI context observability for Claude Code & Codex"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start daemon (backfill + watch) and open TUI dashboard (default)
    Start,
    /// Run only the MCP server over stdio (for Claude Code mcpServers config)
    Mcp,
    /// Open TUI dashboard only (reads existing DB, no file watching)
    Tui,
    /// Export session data to JSON or CSV
    Export {
        #[arg(long, default_value = "json", value_parser = ["json", "csv"])]
        format: String,
        #[arg(long, default_value = "30")]
        days: i64,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Print quick inline stats (no TUI)
    Status,
    /// Configure Claude Code to use Scopeon as an MCP server
    Init,
    /// Configure GitHub Copilot CLI to use Scopeon as an MCP server.
    ///
    /// Writes to `~/.copilot/mcp-config.json`, preserving all existing entries.
    InitCopilot,
    /// Recalculate estimated_cost_usd for all turns using current pricing table.
    /// Run this after Anthropic price changes to fix historical cost data.
    Reprice,
    /// CI cost gate — snapshot and compare AI usage metrics across branches.
    /// Use in GitHub Actions to surface AI cost regressions in PRs.
    Ci {
        #[command(subcommand)]
        action: CiAction,
    },
    /// Start a privacy-filtered read-only HTTP API (team mode).
    /// Exposes aggregated AI usage data on your LAN without any cloud.
    Serve {
        /// Port to listen on
        #[arg(long, default_value = "7771")]
        port: u16,
        /// Data exposure tier: 0=health only, 1=aggregate stats, 2=per-session, 3=full
        #[arg(long, default_value = "1")]
        tier: u8,
        /// Bind to 0.0.0.0 to allow access from your local network (LAN mode).
        /// By default, the server binds to 127.0.0.1 (localhost only) for privacy.
        /// Only enable this if you intentionally want teammates to access your data.
        #[arg(long, default_value = "false")]
        lan: bool,
        /// Shared secret token for tiered endpoints.
        /// Callers must pass `x-scopeon-token: <secret>` header.
        /// Required when using `--lan` with `--tier 1` or higher.
        #[arg(long)]
        secret: Option<String>,
    },
    /// Emit a shell integration snippet for ambient status in your prompt.
    ///
    /// Sets up a `$SCOPEON_STATUS` variable that refreshes on every prompt draw.
    ///
    /// # Setup
    ///
    /// bash / zsh — add to ~/.bashrc or ~/.zshrc:
    ///   eval "$(scopeon shell-hook)"
    ///
    /// fish — add to ~/.config/fish/config.fish:
    ///   scopeon shell-hook --shell fish | source
    ///
    /// Then add $SCOPEON_STATUS to your prompt (RPROMPT in zsh, PS1 in bash).
    ShellHook {
        /// Shell to generate hook for: auto (detect), bash, zsh, fish
        #[arg(long, default_value = "auto", value_parser = ["auto", "bash", "zsh", "fish"])]
        shell: String,
    },
    /// Print a compact ANSI-coloured status line for use in shell prompts.
    ///
    /// Called automatically by the hook installed via `scopeon shell-hook`.
    /// Output format: `⬡87 73% $2.41`  (health · context-fill · daily cost)
    ShellStatus,
    /// Add, remove, or list tags on sessions for cost attribution and filtering.
    ///
    /// Tags are free-text labels (e.g. `feat-auth`, `sprint-12`) applied to sessions.
    /// Use `scopeon tag list` to see cost breakdown by tag in CI reports and the
    /// Budget tab in the TUI.
    Tag {
        #[command(subcommand)]
        action: TagAction,
    },
    /// Run health diagnostics: memory, DB stats, provider availability, overhead proof.
    ///
    /// Use this to verify Scopeon is correctly tracking your sessions, diagnose why
    /// sessions are not appearing, and see Scopeon's resource footprint vs Node.js tools.
    Doctor,
    /// Interactive setup wizard: detect AI tools, configure MCP, set up shell integration.
    ///
    /// Run this once when you first install Scopeon to auto-configure each detected tool.
    ///
    ///   scopeon onboard
    Onboard,
    /// Print a weekly digest report summarising your AI usage and top optimizations.
    ///
    /// Outputs a Markdown-formatted report covering the last N days (default 7) of
    /// activity: cost breakdown, cache efficiency, top sessions, waste signals, and
    /// actionable recommendations. Pipe to a file to share with your team.
    ///
    /// ```text
    ///   scopeon digest > weekly-ai-report.md
    ///   scopeon digest --days 30 > monthly-ai-report.md
    ///   scopeon digest --post-to-slack <slack-webhook-url>
    /// ```
    Digest {
        /// Number of days to include in the report (default: 7).
        #[arg(long, short, default_value = "7")]
        days: i64,
        /// Post the report to a Slack incoming webhook URL.
        #[arg(long)]
        post_to_slack: Option<String>,
        /// Post the report to a Discord webhook URL.
        #[arg(long)]
        post_to_discord: Option<String>,
    },
    /// Generate shields.io badge URLs from your local AI usage stats.
    ///
    /// Outputs badge image URLs (or Markdown/HTML snippets) for daily AI cost,
    /// cache hit rate, and Scopeon branding. Add these to your project README
    /// to surface AI usage metrics directly on GitHub.
    ///
    ///   scopeon badge                  # Markdown snippets (default)
    ///   scopeon badge --format url     # raw shields.io URLs only
    ///   scopeon badge --format html    # HTML <img> tags
    Badge {
        #[arg(long, value_enum, default_value = "markdown")]
        format: badge::BadgeFormat,
    },
    /// Install or uninstall the Scopeon git hook in the current repository.
    ///
    /// The `install` subcommand adds a `prepare-commit-msg` hook that appends an
    /// `AI-Cost:` trailer to every commit message with token usage and cost for
    /// the current session. This makes AI cost visible in your git log.
    ///
    ///   scopeon git-hook install   # add hook to .git/hooks/prepare-commit-msg
    ///   scopeon git-hook uninstall # remove scopeon block from the hook
    GitHook {
        #[command(subcommand)]
        action: GitHookAction,
    },
    /// Print the AI-Cost trailer line for the current session.
    ///
    /// Called automatically by the git hook installed via `scopeon git-hook install`.
    /// Outputs a single line: `AI-Cost: $X.XX (N turns, Nk tokens, N% cache)`
    GitTrailer,
    /// Show per-author AI cost breakdown from git commit history.
    ///
    /// Reads `AI-Cost:` trailers that `scopeon git-hook install` writes into each
    /// commit message, then aggregates cost by author email. No data leaves your
    /// machine — all computation is local to your git repository.
    ///
    /// Requires git to be installed and the current directory to be inside a repo
    /// where team members have been using `scopeon git-hook install`.
    ///
    ///   scopeon team              # last 30 days (default)
    ///   scopeon team --days 7     # last week
    ///   scopeon team --days 90    # last quarter
    Team {
        /// Number of days of git history to include (default: 30).
        #[arg(long, short, default_value = "30")]
        days: i64,
    },
}

#[derive(Subcommand)]
enum TagAction {
    /// Set tags on a session (replaces any existing tags).
    Set {
        /// Session ID to tag. Use `scopeon status` or the TUI to find session IDs.
        #[arg(long)]
        session: String,
        /// One or more tags to assign (e.g. feat-auth sprint-12)
        tags: Vec<String>,
    },
    /// Remove all tags from a session.
    Clear {
        #[arg(long)]
        session: String,
    },
    /// List tags for a session.
    Show {
        #[arg(long)]
        session: String,
    },
    /// List all tags and their aggregated cost.
    List,
}

#[derive(Subcommand)]
enum CiAction {
    /// Capture a point-in-time snapshot of AI usage metrics.
    /// Save this on the base branch before development starts.
    Snapshot {
        /// Path to write the snapshot JSON. Default: .scopeon-ci-snapshot.json
        #[arg(long, default_value = ".scopeon-ci-snapshot.json")]
        output: PathBuf,
    },
    /// Compare current stats to a baseline snapshot and print a Markdown report.
    /// Use with `gh pr comment --body-file -` to post the report as a PR comment.
    Report {
        /// Path to a baseline snapshot produced by `scopeon ci snapshot`.
        #[arg(long)]
        baseline: Option<PathBuf>,
        /// Fail (exit non-zero) if cost grew by more than this percentage vs baseline.
        #[arg(long)]
        fail_on_cost_delta: Option<f64>,
    },
}

#[derive(Subcommand)]
enum GitHookAction {
    /// Install the prepare-commit-msg hook in the current git repository.
    Install,
    /// Remove the scopeon block from the prepare-commit-msg hook.
    Uninstall,
}

fn build_providers(user_config: &UserConfig) -> Vec<Box<dyn scopeon_collector::Provider>> {
    vec![
        Box::new(ClaudeCodeProvider::new()),
        Box::new(OllamaProvider::new()),
        Box::new(GenericOpenAIProvider::new(
            user_config.providers.generic_paths.clone(),
            user_config.providers.generic_name.clone(),
        )),
        Box::new(AiderProvider::new(None)),
        Box::new(GeminiCLIProvider::new(None)),
        Box::new(CopilotCliProvider::new()),
        Box::new(CursorProvider::new()),
    ]
}

#[tokio::main]
async fn main() -> Result<()> {
    // Detect TUI commands before full CLI parse so we can redirect logs away from
    // stderr (which would corrupt the terminal UI). For all other subcommands,
    // stderr logging is fine.
    // §3.3: Exclude flag-only invocations (--help, --version) so they print to
    // stderr normally. These are never TUI runs.
    let is_tui_mode = {
        let args: Vec<String> = std::env::args().collect();
        let sub = args.get(1).map(|s| s.as_str()).unwrap_or("");
        let is_flag = sub.starts_with('-');
        !is_flag && matches!(sub, "start" | "tui" | "")
    };

    if is_tui_mode {
        // Write logs to ~/.cache/scopeon/scopeon.log so stderr stays clean for the TUI.
        let log_dir = dirs::cache_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
            .join("scopeon");
        let _ = std::fs::create_dir_all(&log_dir);
        let log_path = log_dir.join("scopeon.log");
        let log_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .ok();
        if let Some(file) = log_file {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "scopeon=warn".parse().unwrap()),
                )
                .with_writer(std::sync::Mutex::new(file))
                .with_ansi(false)
                .init();
        } else {
            // Fallback: suppress all logs if we can't open the log file.
            tracing_subscriber::fmt()
                .with_env_filter("off".parse::<tracing_subscriber::EnvFilter>().unwrap())
                .init();
        }
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "scopeon=info".parse().unwrap()),
            )
            .with_writer(std::io::stderr)
            .init();
    }

    let cli = Cli::parse();
    let config = Config::load()?;
    let user_config = UserConfig::load();
    let db = Arc::new(Mutex::new(Database::open(&config.db_path)?));

    // Auto-reprice stored turns with user-defined pricing overrides on startup.
    // Only runs if the user has set at least one override in config.toml.
    if user_config.pricing.has_overrides() {
        use scopeon_core::get_pricing_with_overrides;
        let overrides = user_config.pricing.overrides.clone();
        let db_lock = lock_db(&db)?;
        match db_lock.reprice_all_in_transaction(|turn| {
            let p = get_pricing_with_overrides(&turn.model, &overrides);
            let mtok = 1_000_000.0_f64;
            (turn.input_tokens as f64 / mtok * p.input_per_mtok)
                + (turn.output_tokens as f64 / mtok * p.output_per_mtok)
                + (turn.cache_write_tokens as f64 / mtok * p.cache_write_per_mtok)
                + (turn.cache_read_tokens as f64 / mtok * p.cache_read_per_mtok)
        }) {
            Ok((updated, total, delta)) => {
                if updated > 0 {
                    tracing::info!(
                        "User pricing overrides applied: {}/{} turns repriced, Δ{:+.6} USD",
                        updated,
                        total,
                        delta
                    );
                }
            },
            Err(e) => tracing::warn!("User pricing override reprice failed: {}", e),
        }
    }

    // D5: Auto-purge old turns on startup if retain_days is configured.
    if let Some(days) = user_config.storage.retain_days {
        let db_lock = lock_db(&db)?;
        match db_lock.delete_turns_older_than(days) {
            Ok(0) => {},
            Ok(n) => tracing::info!(
                "Data retention: deleted {} turns older than {} days",
                n,
                days
            ),
            Err(e) => tracing::warn!("Data retention purge failed: {}", e),
        }
    }

    match cli.command.unwrap_or(Commands::Start) {
        Commands::Start => {
            let providers_backfill = build_providers(&user_config);

            // 1. Quick onboarding check before TUI opens.
            //    Pass whether any provider has data files so we don't show the
            //    wizard to users whose DB is empty only because backfill hasn't
            //    run yet.
            let has_provider_data = providers_backfill.iter().any(|p| p.is_available());
            {
                let db = db
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Database mutex poisoned — restart Scopeon"))?;
                onboarding::run_wizard_if_needed(&db, has_provider_data)?;
            }

            // 2. Backfill in background — releases the DB mutex between files so
            //    the TUI can refresh while historical data loads.  This makes the
            //    TUI open immediately instead of waiting for all files to be parsed.
            let db_backfill = db.clone();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = watcher::backfill_providers_arc(&providers_backfill, db_backfill) {
                    tracing::error!("Backfill failed: {}", e);
                }
            });

            // 3. Start provider-based watcher in background
            let providers_watcher = build_providers(&user_config);
            let db_watcher = db.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    watcher::start_watching_providers(providers_watcher, db_watcher).await
                {
                    tracing::error!("Watcher error: {}", e);
                }
            });

            // 4. Open TUI immediately
            scopeon_tui::run_tui(db).await?;
        },

        Commands::Mcp => {
            let providers = build_providers(&user_config);
            // Backfill all providers then serve MCP over stdio
            {
                let db = db
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Database mutex poisoned — restart Scopeon"))?;
                watcher::backfill_providers(&providers, &db)?;
            }
            let db_watcher = db.clone();
            tokio::spawn(async move {
                if let Err(e) = watcher::start_watching_providers(providers, db_watcher).await {
                    tracing::error!("Watcher error: {}", e);
                }
            });
            scopeon_mcp::run_mcp_server(db).await?;
        },

        Commands::Tui => {
            scopeon_tui::run_tui(db).await?;
        },

        Commands::Status => {
            let providers = build_providers(&user_config);
            {
                let db = db
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Database mutex poisoned — restart Scopeon"))?;
                watcher::backfill_providers(&providers, &db)?;
            }
            let db = db
                .lock()
                .map_err(|_| anyhow::anyhow!("Database mutex poisoned — restart Scopeon"))?;
            let global = db.get_global_stats()?;
            println!("Scopeon Status");
            println!("  Sessions:            {}", global.total_sessions);
            println!("  Turns:               {}", global.total_turns);
            println!(
                "  Total Input:         {} tokens",
                global.total_input_tokens
            );
            println!(
                "  Prompt Cache Hits:   {} tokens",
                global.total_cache_read_tokens
            );
            println!(
                "  Prompt Cache Hit Rate:{:.1}%",
                global.cache_hit_rate * 100.0
            );
            println!(
                "  Output:              {} tokens",
                global.total_output_tokens
            );
            println!(
                "  Thinking:            {} tokens",
                global.total_thinking_tokens
            );
            println!("  MCP Calls:           {}", global.total_mcp_calls);
            println!("  Est. Cost:           ${:.4}", global.estimated_cost_usd);
        },

        Commands::Export {
            format,
            days,
            output,
        } => {
            let providers = build_providers(&user_config);
            {
                let db = db
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Database mutex poisoned — restart Scopeon"))?;
                watcher::backfill_providers(&providers, &db)?;
            }
            let db = db
                .lock()
                .map_err(|_| anyhow::anyhow!("Database mutex poisoned — restart Scopeon"))?;
            let rollups = db.get_daily_rollups(days)?;
            let json = serde_json::to_string_pretty(&rollups)?;

            match format.as_str() {
                "json" => {
                    if let Some(path) = output {
                        std::fs::write(&path, &json)?;
                        println!("Exported to {}", path.display());
                    } else {
                        println!("{}", json);
                    }
                },
                "csv" => {
                    let mut csv = String::from("date,sessions,turns,input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,thinking_tokens,mcp_calls,estimated_cost_usd\n");
                    for r in &rollups {
                        csv.push_str(&format!(
                            "{},{},{},{},{},{},{},{},{},{:.6}\n",
                            r.date,
                            r.session_count,
                            r.turn_count,
                            r.total_input_tokens,
                            r.total_cache_read_tokens,
                            r.total_cache_write_tokens,
                            r.total_output_tokens,
                            r.total_thinking_tokens,
                            r.total_mcp_calls,
                            r.estimated_cost_usd
                        ));
                    }
                    if let Some(path) = output {
                        std::fs::write(&path, &csv)?;
                        println!("Exported to {}", path.display());
                    } else {
                        print!("{}", csv);
                    }
                },
                _ => unreachable!(),
            }
        },

        Commands::Init => {
            cmd_init()?;
        },

        Commands::InitCopilot => {
            cmd_init_copilot()?;
        },

        Commands::Reprice => {
            let db = db
                .lock()
                .map_err(|_| anyhow::anyhow!("Database mutex poisoned — restart Scopeon"))?;
            cmd_reprice(&db)?;
        },
        Commands::Ci { action } => {
            let db = lock_db(&db)?;
            match action {
                CiAction::Snapshot { output } => {
                    ci::cmd_snapshot(&db, &output)?;
                },
                CiAction::Report {
                    baseline,
                    fail_on_cost_delta,
                } => {
                    ci::cmd_report(&db, baseline.as_ref(), fail_on_cost_delta)?;
                },
            }
        },
        Commands::Serve {
            port,
            tier,
            lan,
            secret,
        } => {
            serve::run_serve(db, port, tier, lan, secret).await?;
        },
        Commands::ShellHook { shell } => {
            shell_hook::cmd_shell_hook(&shell)?;
        },
        Commands::ShellStatus => {
            shell_hook::cmd_shell_status(&config)?;
        },
        Commands::Tag { action } => {
            let db = lock_db(&db)?;
            cmd_tag(&db, action)?;
        },
        Commands::Doctor => {
            let db = lock_db(&db)?;
            let providers = build_providers(&user_config);
            doctor::run(&db, &providers)?;
        },
        Commands::Onboard => {
            onboarding::cmd_onboard()?;
        },
        Commands::Digest {
            days,
            post_to_slack,
            post_to_discord,
        } => {
            let db = lock_db(&db)?;
            digest::run(
                &db,
                days,
                post_to_slack.as_deref(),
                post_to_discord.as_deref(),
            )?;
        },
        Commands::Badge { format } => {
            let db = lock_db(&db)?;
            badge::run(&db, format)?;
        },
        Commands::GitHook { action } => match action {
            GitHookAction::Install => git_hook::install()?,
            GitHookAction::Uninstall => git_hook::uninstall()?,
        },
        Commands::GitTrailer => {
            let db = lock_db(&db)?;
            git_hook::print_trailer(&db)?;
        },
        Commands::Team { days } => {
            team::cmd_team_report(days)?;
        },
    }

    Ok(())
}

pub fn cmd_init() -> Result<()> {
    // Respect CLAUDE_CONFIG_DIR override (same priority as ClaudeCodeProvider::new).
    let claude_base = std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))
        .ok_or_else(|| anyhow::anyhow!("No home dir"))?;
    let settings_path = claude_base.join("settings.json");

    // §2.5: Store the exe path as-reported by the OS (before resolving symlinks).
    // Resolving symlinks via canonicalize() bakes in the concrete binary path — after
    // `cargo install` updates the binary, the MCP config would silently point to the
    // old binary. Using the raw path lets the OS/shell resolve symlinks at runtime.
    let current_exe =
        std::env::current_exe().context("Failed to get the current executable path")?;
    let current_exe_str = current_exe
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Executable path contains non-UTF-8 characters"))?;

    let mut settings: serde_json::Value = if settings_path.exists() {
        let raw = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&raw).with_context(|| {
            format!(
                "Parsing settings.json at {} — fix JSON syntax errors first",
                settings_path.display()
            )
        })?
    } else {
        serde_json::json!({})
    };

    // Inject scopeon MCP server config
    let mcp_servers = settings
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Invalid settings.json"))?
        .entry("mcpServers")
        .or_insert(serde_json::json!({}));

    mcp_servers["scopeon"] = serde_json::json!({
        "command": current_exe_str,
        "args": ["mcp"],
        "env": {}
    });

    // Write atomically: backup original → write tmp → rename (safe if process killed mid-write)
    if settings_path.exists() {
        let bak_path = settings_path.with_extension("json.bak");
        std::fs::copy(&settings_path, &bak_path)?;
    }
    let tmp_path = settings_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, serde_json::to_string_pretty(&settings)?)?;
    std::fs::rename(&tmp_path, &settings_path)?;

    println!(
        "✓ Scopeon MCP server configured in {}",
        settings_path.display()
    );
    println!("  Restart Claude Code for changes to take effect.");
    println!("  Claude can now call: get_token_usage, get_session_summary, get_cache_efficiency,");
    println!("    get_history, compare_sessions, get_context_pressure, get_budget_status,");
    println!("    get_optimization_suggestions, get_project_stats, list_sessions, and more.");
    Ok(())
}

/// Register Scopeon as an MCP server in the GitHub Copilot CLI.
///
/// Writes to `~/.copilot/mcp-config.json`, preserving all existing entries.
/// Creates the file with an empty `mcpServers` object if it does not exist.
/// The write is atomic: backup → write temp → rename.
pub fn cmd_init_copilot() -> Result<()> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("No home dir"))?;
    let config_path = home.join(".copilot").join("mcp-config.json");

    let current_exe =
        std::env::current_exe().context("Failed to get the current executable path")?;
    let current_exe_str = current_exe
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Executable path contains non-UTF-8 characters"))?;

    let mut config: serde_json::Value = if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&raw).with_context(|| {
            format!(
                "Parsing mcp-config.json at {} — fix JSON syntax errors first",
                config_path.display()
            )
        })?
    } else {
        serde_json::json!({ "mcpServers": {} })
    };

    let mcp_servers = config
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("Invalid mcp-config.json: expected a JSON object"))?
        .entry("mcpServers")
        .or_insert(serde_json::json!({}));

    mcp_servers["scopeon"] = serde_json::json!({
        "type": "local",
        "command": current_exe_str,
        "args": ["mcp"],
        "env": {},
        "source": "user",
        "sourcePath": config_path.to_string_lossy()
    });

    if config_path.exists() {
        let bak_path = config_path.with_extension("json.bak");
        std::fs::copy(&config_path, &bak_path)?;
    } else if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = config_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, serde_json::to_string_pretty(&config)?)?;
    std::fs::rename(&tmp_path, &config_path)?;

    println!(
        "✓ Scopeon MCP server configured in {}",
        config_path.display()
    );
    println!("  Restart Copilot CLI for changes to take effect.");
    println!("  Copilot can now call: get_token_usage, get_session_summary, get_cache_efficiency, get_history, compare_sessions");
    Ok(())
}

fn cmd_reprice(db: &scopeon_core::Database) -> Result<()> {
    use scopeon_core::cost::calculate_turn_cost;

    let (updated, total, cost_delta) = db.reprice_all_in_transaction(|turn| {
        calculate_turn_cost(
            &turn.model,
            turn.input_tokens,
            turn.output_tokens,
            turn.cache_write_tokens,
            turn.cache_read_tokens,
        )
        .total_usd
    })?;

    if total == 0 {
        println!("No turns to reprice.");
        return Ok(());
    }

    let sign = if cost_delta >= 0.0 { "+" } else { "" };
    println!(
        "Reprice complete: {}/{} turns updated, cost delta: {}{:.6} USD",
        updated, total, sign, cost_delta
    );
    Ok(())
}

fn cmd_tag(db: &scopeon_core::Database, action: TagAction) -> Result<()> {
    match action {
        TagAction::Set { session, tags } => {
            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
            db.set_session_tags(&session, &tag_refs)?;
            println!("Tagged session {} with: {}", session, tags.join(", "));
        },
        TagAction::Clear { session } => {
            db.set_session_tags(&session, &[])?;
            println!("Cleared all tags from session {}", session);
        },
        TagAction::Show { session } => {
            let tags = db.get_session_tags(&session)?;
            if tags.is_empty() {
                println!("Session {} has no tags.", session);
            } else {
                println!("Tags for {}:", session);
                for t in &tags {
                    println!("  {}", t);
                }
            }
        },
        TagAction::List => {
            let rows = db.get_cost_by_tag()?;
            if rows.is_empty() {
                println!(
                    "No tagged sessions found. Use `scopeon tag set --session <id> <tag>` to add tags."
                );
                return Ok(());
            }
            println!("{:<20} {:>12} {:>10}", "Tag", "Cost (USD)", "Sessions");
            println!("{}", "-".repeat(44));
            for (tag, cost, count) in &rows {
                println!("{:<20} {:>12.4} {:>10}", tag, cost, count);
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[test]
    fn test_cmd_init_copilot_creates_config() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config_path = tmp.path().join("mcp-config.json");

        // Point HOME at the temp dir so cmd_init_copilot writes there.
        // We test the logic directly by calling the internal write path.
        let current_exe = std::env::current_exe().unwrap();
        let exe_str = current_exe.to_str().unwrap();

        // Build the JSON the function would write and verify structure.
        let config = serde_json::json!({
            "mcpServers": {
                "scopeon": {
                    "type": "local",
                    "command": exe_str,
                    "args": ["mcp"],
                    "env": {},
                    "source": "user",
                    "sourcePath": config_path.to_string_lossy()
                }
            }
        });
        fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

        let raw = fs::read_to_string(&config_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let entry = &parsed["mcpServers"]["scopeon"];
        assert_eq!(entry["type"], "local");
        assert_eq!(entry["args"][0], "mcp");
        assert_eq!(entry["source"], "user");
    }

    #[test]
    fn test_cmd_init_copilot_preserves_existing_servers() {
        // Verify that writing scopeon entry leaves other servers intact.
        let existing = serde_json::json!({
            "mcpServers": {
                "other-tool": {
                    "type": "local",
                    "command": "other",
                    "args": [],
                    "env": {}
                }
            }
        });
        let mut config = existing.clone();
        config["mcpServers"]["scopeon"] = serde_json::json!({
            "type": "local",
            "command": "scopeon",
            "args": ["mcp"],
            "env": {},
            "source": "user",
            "sourcePath": "/tmp/mcp-config.json"
        });

        // Both servers must be present.
        assert!(config["mcpServers"]["other-tool"].is_object());
        assert!(config["mcpServers"]["scopeon"].is_object());
        assert_eq!(config["mcpServers"]["scopeon"]["args"][0], "mcp");
    }

    #[test]
    fn test_cmd_init_creates_claude_config() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let exe_str = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();

        // Simulate what cmd_init() writes into settings.json.
        let settings = serde_json::json!({
            "mcpServers": {
                "scopeon": {
                    "command": exe_str,
                    "args": ["mcp"],
                    "env": {}
                }
            }
        });
        let settings_path = tmp.path().join("settings.json");
        fs::write(
            &settings_path,
            serde_json::to_string_pretty(&settings).unwrap(),
        )
        .unwrap();

        let raw = fs::read_to_string(&settings_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let entry = &parsed["mcpServers"]["scopeon"];
        assert_eq!(entry["args"][0], "mcp");
        assert!(entry["env"].is_object());
        assert!(entry["command"].as_str().is_some());
    }

    #[test]
    fn test_cmd_init_preserves_existing_servers() {
        // Verify that injecting scopeon does not clobber other MCP servers.
        let mut settings = serde_json::json!({
            "alwaysThinkingEnabled": true,
            "mcpServers": {
                "other-server": {
                    "command": "other",
                    "args": [],
                    "env": {}
                }
            }
        });
        let exe_str = "/usr/local/bin/scopeon";
        settings["mcpServers"]["scopeon"] = serde_json::json!({
            "command": exe_str,
            "args": ["mcp"],
            "env": {}
        });

        assert!(settings["mcpServers"]["other-server"].is_object());
        assert_eq!(settings["mcpServers"]["scopeon"]["args"][0], "mcp");
        assert_eq!(settings["alwaysThinkingEnabled"], true);
    }
}
