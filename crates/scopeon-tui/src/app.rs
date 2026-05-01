use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event,
    KeyCode, KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear as TermClear, ClearType, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use crossterm::{cursor, ExecutableCommand, QueueableCommand};
use ratatui::{backend::CrosstermBackend, Terminal};

use chrono::{Datelike, Timelike};
use scopeon_core::{
    apply_provider_preset, list_provider_optimization_reports, AgentNode, Database, GlobalStats,
    InteractionEvent, OptimizationPresetId, OptimizationProviderId, ProjectStats, Session,
    SessionAnomaly, SessionStats, SessionSummary, TaskRun, ToolBreakdownItem, ToolCall, ToolStat,
    UserConfig,
};
use scopeon_metrics::{
    compute_health_score_with_breakdown, MetricCategory, MetricRegistry, MetricValue, Suggestion,
    WasteReport,
};

use crate::theme::Theme;
use crate::ui::draw;

/// Sort order for the Sessions tab.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SessionSort {
    Newest,
    Oldest,
    MostExpensive,
}

impl SessionSort {
    pub fn cycle(self) -> Self {
        match self {
            SessionSort::Newest => SessionSort::Oldest,
            SessionSort::Oldest => SessionSort::MostExpensive,
            SessionSort::MostExpensive => SessionSort::Newest,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            SessionSort::Newest => "Newest",
            SessionSort::Oldest => "Oldest",
            SessionSort::MostExpensive => "Most Expensive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tab {
    Sessions = 0,
    Spend = 1,
    Config = 2,
}

impl Tab {
    pub fn from_index(i: usize) -> Self {
        match i {
            0 => Tab::Sessions,
            1 => Tab::Spend,
            2 => Tab::Config,
            _ => Tab::Sessions,
        }
    }
    pub fn index(self) -> usize {
        self as usize
    }
    pub fn count() -> usize {
        const TABS: &[Tab] = &[Tab::Sessions, Tab::Spend, Tab::Config];
        TABS.len()
    }
}

/// Which pane has keyboard focus in a split-pane tab.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaneFocus {
    Left,
    Right,
}

/// Which section is active in the full-screen session detail view.
/// Cycled with the Tab key.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum DetailSection {
    #[default]
    Turns,
    Context,
    McpSkills,
}

impl DetailSection {
    pub fn next(self) -> Self {
        match self {
            DetailSection::Turns => DetailSection::Context,
            DetailSection::Context => DetailSection::McpSkills,
            DetailSection::McpSkills => DetailSection::Turns,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            DetailSection::Turns => DetailSection::McpSkills,
            DetailSection::Context => DetailSection::Turns,
            DetailSection::McpSkills => DetailSection::Context,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            DetailSection::Turns => "Turns",
            DetailSection::Context => "Context",
            DetailSection::McpSkills => "MCP & Skills",
        }
    }
}

/// Budget spending state computed each refresh cycle.
#[derive(Debug, Clone, Default)]
pub struct BudgetState {
    pub daily_spent: f64,
    pub weekly_spent: f64,
    pub monthly_spent: f64,
    pub daily_limit: f64,
    pub weekly_limit: f64,
    pub monthly_limit: f64,
    pub projected_monthly: f64,
    pub cost_by_model: Vec<(String, f64)>,
    pub cost_by_project: Vec<(String, f64)>,
    pub daily_history: Vec<(String, f64)>, // (date, cost) last 14 days newest-first
    // Context window pressure for the live session's most recent turn
    pub context_pressure_pct: f64,
    pub context_tokens_remaining: i64,
    pub context_window: i64,
    /// Predicted turns remaining before context window is exhausted (linear trend).
    /// `None` when there are fewer than 3 turns or the trend is flat/decreasing.
    pub predicted_turns_remaining: Option<i64>,
    /// Predicted days until the monthly budget limit is exhausted (linear regression on 7d costs).
    /// `None` when no monthly limit is set, fewer than 3 daily entries, or spend is flat/declining.
    pub predicted_days_until_monthly_limit: Option<f64>,
    /// IS-M: Cache efficiency drop for the live session.
    /// Some(pct) when the last 3 turns' average efficiency is less than 50% of the prior 7-turn avg.
    /// `None` when there is no live session or insufficient turn history.
    pub cache_bust_drop: Option<f64>,
    /// IS-E: Bayesian cold-start prior — 90-day median tokens per turn (used for turn 0-2 forecasts).
    pub median_tokens_per_turn: f64,
    /// IS-K: Cost breakdown by auto-detected task type (tag field) for the last 30 days.
    pub cost_by_tag: Vec<(String, f64, i64)>,

    /// C-19: Cost breakdown by provider → model for the Spend tab tree.
    /// Each entry: (provider, model, total_cost). Sorted by provider total then model cost.
    pub cost_by_provider_model: Vec<(String, String, f64)>,

    // IS-14: Spend projection — end-of-day estimate based on current hourly rate.
    pub daily_hourly_rate: f64,   // current spend / hours elapsed today
    pub daily_projected_eod: f64, // extrapolated daily total if pace continues
}

pub struct ProviderStatus {
    pub id: String,
    pub name: String,
    pub is_active: bool,
    pub session_count: usize,
    pub turn_count: usize,
    pub last_update: Option<String>,
    pub config_hint: String,
    pub optimization_status: String,
}

pub struct App {
    pub tab: Tab,
    pub config: UserConfig,
    pub registry: MetricRegistry,

    // Live session
    pub live_stats: Option<SessionStats>,
    pub live_tool_calls: Vec<ToolCall>,
    pub live_interaction_events: Vec<InteractionEvent>,
    pub live_task_runs: Vec<TaskRun>,
    pub is_live: bool,                 // true = last turn < LIVE_THRESHOLD ago
    pub active_sessions: Vec<Session>, // all sessions with recent activity
    pub copilot_active: bool,          // Copilot VS Code log freshness

    // Global
    pub global_stats: Option<GlobalStats>,
    pub global_tool_stats: Vec<ToolStat>,
    pub project_stats: Vec<ProjectStats>,
    pub session_anomalies: Vec<SessionAnomaly>,

    // Session list (Sessions tab)
    pub sessions_list: Vec<Session>,
    pub session_summaries: std::collections::HashMap<String, SessionSummary>,
    pub selected_session_idx: usize,
    pub selected_session_stats: Option<SessionStats>,
    pub selected_session_interaction_events: Vec<InteractionEvent>,
    pub selected_session_task_runs: Vec<TaskRun>,
    pub session_detail_mode: bool,
    pub turn_scroll_detail: usize,
    pub sessions_filter: String,
    pub sessions_filter_active: bool,
    pub sessions_filter_error: Option<String>, // shown in amber when filter has a parse error

    // Providers
    pub providers: Vec<ProviderStatus>,

    // Agent tree (Phase 4)
    pub agent_roots: Vec<AgentNode>,

    // Scroll offsets
    pub history_scroll: usize,
    pub turn_scroll: usize,
    pub projects_scroll: usize,

    // Pane navigation
    pub pane_focus: PaneFocus,

    // Waste & suggestions (computed in refresh, stable across renders)
    pub waste_report: Option<WasteReport>,
    pub suggestions: Vec<Suggestion>,
    /// Personalised waste thresholds derived from historical percentiles.
    pub user_thresholds: scopeon_metrics::UserThresholds,

    // Cached metric rows computed in refresh() — avoids recomputing every 200ms render frame.
    // Each entry: (category_label, name, value, formatted, description)
    pub cached_metrics: Vec<(MetricCategory, String, MetricValue, String, String)>,

    // Health & trends
    pub health_score: f64,
    pub health_breakdown: Option<scopeon_metrics::HealthBreakdown>,
    pub trend_cost_pct: f64,  // % change vs yesterday (positive = worse)
    pub trend_cache_pct: f64, // % change vs yesterday (positive = better)
    pub trend_turns: i64,     // delta vs yesterday

    // Budget
    pub budget: BudgetState,

    // Context pressure alert state (tracks threshold crossings to avoid repeat bells)
    pub context_alert_threshold_crossed: u8, // 0=none, 1=80%, 2=95%
    pub alert_banner: Option<(String, ratatui::style::Color)>,

    // UI state
    pub show_help: bool,
    pub last_refresh: Instant,
    pub refresh_interval: Duration,
    pub quit: bool,
    pub theme: Theme,

    // Animation & interaction state
    pub spinner_frame: usize,
    pub refresh_in_progress: bool,
    pub hint_tick: u64,
    pub sessions_sort: SessionSort,
    pub toast: Option<(String, Instant)>,

    // IS-4: Zen mode — collapses TUI to a single ambient status line.
    // Auto-expands when context > 80% or budget > 90%.
    pub zen_mode: bool,
    /// Set when zen was auto-exited due to pressure. Counts refresh cycles
    /// with ctx < 70% AND budget ratio < 90% before re-entering zen.
    pub zen_auto_exited: bool,
    pub zen_clear_cycles: u8,

    // IS-10: Context heartbeat — pulse_phase increments each refresh tick.
    // Used to animate the context fill bar's leading edge.
    pub pulse_phase: f64,

    // IS-1: Narrative rotation index — cycles through insight sentences in KPI strip.
    pub narrative_idx: usize,

    // IS-2: Temporal replay — index into the selected session's turns list (None = no replay).
    // When Some(i), the fullscreen session detail highlights that specific turn and shows
    // a context/cost snapshot. ← decrements, → increments.
    pub replay_turn_idx: Option<usize>,

    // IS-5: Mouse state — last-clicked session list row, for single/double-click detection.
    pub mouse_last_click_row: Option<u16>,
    /// Last known terminal height — updated each render tick so mouse hit-testing
    /// can compute the correct session-list body row offset.
    pub terminal_height: u16,
    /// Last known terminal width — used to gate split-panel preview to the left column.
    pub terminal_width: u16,

    // C-17: Provider/model scope — filters the Sessions tab list.
    // `scope_provider` is None for "All". `scope_model` is None for "All models".
    pub scope_provider: Option<String>,
    pub scope_model: Option<String>,
    /// Unique providers across all sessions (populated each refresh).
    pub all_providers: Vec<String>,
    /// Unique models for the currently scoped provider (or all providers when None).
    pub all_models: Vec<String>,

    // C-10: Command palette state.
    pub command_palette_active: bool,
    pub command_palette_query: String,

    /// Active section in full-screen session detail (cycled with Tab).
    pub detail_section: DetailSection,
    /// Tool/MCP/skills breakdown for the currently selected session.
    /// Cleared when selected session changes; loaded in refresh().
    pub selected_session_tools: Option<Vec<ToolBreakdownItem>>,
    /// Whether to show the Trends BarChart instead of KPI cards (toggled with `t`).
    pub show_trends: bool,

    // Config tab state
    pub config_providers: Vec<ConfigProvider>,
    pub config_selected_idx: usize,
    pub config_preset_selector_active: bool,
    pub config_preset_selected_idx: usize,
}

#[derive(Debug, Clone)]
pub struct ConfigProvider {
    pub id: String,
    pub display_name: String,
    pub detected: bool,
    pub support: String,
    pub current_preset: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let config = UserConfig::load();
        let refresh_interval = Duration::from_secs(config.general.refresh_interval_secs);
        let theme = Theme::from_name(&config.general.theme);
        App {
            tab: Tab::Sessions,
            config,
            registry: MetricRegistry::default(),
            live_stats: None,
            live_tool_calls: Vec::new(),
            live_interaction_events: Vec::new(),
            live_task_runs: Vec::new(),
            is_live: false,
            active_sessions: Vec::new(),
            copilot_active: false,
            global_stats: None,
            global_tool_stats: Vec::new(),
            project_stats: Vec::new(),
            session_anomalies: Vec::new(),
            sessions_list: Vec::new(),
            session_summaries: std::collections::HashMap::new(),
            selected_session_idx: 0,
            selected_session_stats: None,
            selected_session_interaction_events: Vec::new(),
            selected_session_task_runs: Vec::new(),
            session_detail_mode: false,
            turn_scroll_detail: 0,
            sessions_filter: String::new(),
            sessions_filter_active: false,
            sessions_filter_error: None,
            providers: Vec::new(),
            agent_roots: Vec::new(),
            history_scroll: 0,
            turn_scroll: 0,
            projects_scroll: 0,
            pane_focus: PaneFocus::Left,
            waste_report: None,
            suggestions: Vec::new(),
            user_thresholds: scopeon_metrics::UserThresholds::default(),
            cached_metrics: Vec::new(),
            health_score: 0.0,
            health_breakdown: None,
            trend_cost_pct: 0.0,
            trend_cache_pct: 0.0,
            trend_turns: 0,
            budget: BudgetState::default(),
            context_alert_threshold_crossed: 0,
            alert_banner: None,
            show_help: false,
            last_refresh: Instant::now() - Duration::from_secs(10),
            refresh_interval,
            quit: false,
            theme,
            spinner_frame: 0,
            refresh_in_progress: false,
            hint_tick: 0,
            sessions_sort: SessionSort::Newest,
            toast: None,
            zen_mode: false,
            zen_auto_exited: false,
            zen_clear_cycles: 0,
            pulse_phase: 0.0,
            narrative_idx: 0,
            replay_turn_idx: None,
            mouse_last_click_row: None,
            terminal_height: 24,
            terminal_width: 220,
            scope_provider: None,
            scope_model: None,
            all_providers: Vec::new(),
            all_models: Vec::new(),
            command_palette_active: false,
            command_palette_query: String::new(),
            detail_section: DetailSection::Turns,
            selected_session_tools: None,
            show_trends: false,
            config_providers: Vec::new(),
            config_selected_idx: 0,
            config_preset_selector_active: false,
            config_preset_selected_idx: 0,
        }
    }

    pub fn refresh(&mut self, db: &Database) {
        // ── Live session (staleness-aware) ────────────────────────────────────
        const LIVE_THRESHOLD_MS: i64 = 15 * 60 * 1_000; // 15 minutes
        let now_ms = chrono::Utc::now().timestamp_millis();

        // Load most-recent session regardless (we always want to show *something*)
        if let Ok(Some(sid)) = db.get_latest_session_id() {
            self.live_stats = db.get_session_stats(&sid).ok();
            self.live_tool_calls = db.list_tool_calls_for_session(&sid).unwrap_or_default();
            self.live_interaction_events = db
                .list_interaction_events_for_session(&sid, 10_000)
                .unwrap_or_default();
            self.live_task_runs = db.list_task_runs_for_session(&sid).unwrap_or_default();
        } else {
            self.live_stats = None;
            self.live_tool_calls.clear();
            self.live_interaction_events.clear();
            self.live_task_runs.clear();
        }

        // Determine staleness from last_turn_at
        self.is_live = self
            .live_stats
            .as_ref()
            .and_then(|s| s.session.as_ref())
            .map(|s| now_ms - s.last_turn_at < LIVE_THRESHOLD_MS)
            .unwrap_or(false);

        // Detect Copilot activity via VS Code log freshness
        self.copilot_active = detect_copilot_activity();

        // ── Global stats ─────────────────────────────────────────────────────
        self.global_stats = db.get_global_stats().ok();
        let provider_db_stats = db.get_stats_by_provider().unwrap_or_default();
        self.providers = build_provider_status(&provider_db_stats, &self.config);
        self.project_stats = db.get_project_stats().unwrap_or_default();
        self.session_anomalies = db.get_session_anomalies().unwrap_or_default();
        self.global_tool_stats = db.get_tool_stats(None).unwrap_or_default();

        // ── Agent trees ───────────────────────────────────────────────────────
        self.agent_roots = db
            .get_agent_root_ids()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|root_id| db.get_agent_tree(&root_id).ok())
            .collect();

        // ── Session list (for Sessions tab) ──────────────────────────────────
        self.sessions_list = db.list_sessions(200).unwrap_or_default();
        self.session_summaries = db
            .list_session_summaries(200)
            .unwrap_or_default()
            .into_iter()
            .map(|s| (s.session_id.clone(), s))
            .collect();

        // Recompute active sessions now that sessions_list is loaded
        self.active_sessions = self
            .sessions_list
            .iter()
            .filter(|s| now_ms - s.last_turn_at < LIVE_THRESHOLD_MS)
            .cloned()
            .collect();
        // Clamp selection index
        if !self.sessions_list.is_empty() {
            self.selected_session_idx = self.selected_session_idx.min(self.sessions_list.len() - 1);
            // Load selected session stats (lazy: only if not in detail mode or just switched)
            let sel_id = &self.sessions_list[self.selected_session_idx].id;
            if self
                .selected_session_stats
                .as_ref()
                .and_then(|s| s.session.as_ref())
                .map(|s| &s.id)
                != Some(sel_id)
            {
                self.selected_session_stats = db.get_session_stats(sel_id).ok();
                // Also reload tool breakdown whenever stats change.
                self.selected_session_tools = db
                    .get_session_tool_breakdown(sel_id)
                    .ok()
                    .filter(|v| !v.is_empty());
            }
            self.selected_session_interaction_events = db
                .list_interaction_events_for_session(sel_id, 10_000)
                .unwrap_or_default();
            self.selected_session_task_runs =
                db.list_task_runs_for_session(sel_id).unwrap_or_default();
        } else {
            self.selected_session_stats = None;
            self.selected_session_tools = None;
            self.selected_session_interaction_events.clear();
            self.selected_session_task_runs.clear();
        }

        // ── Adaptive thresholds (from historical daily_rollup) ───────────────
        if let Ok(threshold_data) = db.get_threshold_data() {
            self.user_thresholds =
                scopeon_metrics::UserThresholds::from_daily_data(&threshold_data);
        }

        // ── Waste + suggestions + cached metrics ─────────────────────────────
        if let Some(stats) = &self.live_stats {
            let daily = self
                .global_stats
                .as_ref()
                .map(|g| g.daily.as_slice())
                .unwrap_or(&[]);
            let ctx = scopeon_metrics::MetricContext {
                turns: &stats.turns,
                session: stats.session.as_ref(),
                daily_rollups: daily,
                provider_name: stats
                    .session
                    .as_ref()
                    .map(|s| s.provider.as_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or("unknown"),
                tool_calls: &self.live_tool_calls,
                interaction_events: &self.live_interaction_events,
                task_runs: &self.live_task_runs,
            };
            let waste = WasteReport::compute_with_thresholds(&ctx, &self.user_thresholds);
            let (score, breakdown) = compute_health_score_with_breakdown(&ctx, &waste);
            self.health_score = score;
            self.health_breakdown = Some(breakdown);
            self.suggestions =
                scopeon_metrics::compute_suggestions(&ctx, &waste, self.global_stats.as_ref());
            self.waste_report = Some(waste);

            // Cache metric computations so the render loop never calls compute() per frame
            let cats = [
                MetricCategory::Cache,
                MetricCategory::Cost,
                MetricCategory::Velocity,
                MetricCategory::Quality,
                MetricCategory::Pattern,
            ];
            self.cached_metrics.clear();
            for cat in &cats {
                for m in self.registry.by_category(cat.clone()) {
                    let val = m.compute(&ctx);
                    let fmted = m.format(&val);
                    self.cached_metrics.push((
                        cat.clone(),
                        m.name().to_string(),
                        val,
                        fmted,
                        m.description().to_string(),
                    ));
                }
            }
        } else {
            self.waste_report = None;
            self.suggestions = Vec::new();
            self.health_score = 0.0;
            self.health_breakdown = None;
            self.cached_metrics.clear();
        }

        // ── Trend indicators (vs yesterday) ───────────────────────────────────
        if let Some(global) = &self.global_stats {
            if global.daily.len() >= 2 {
                let today = &global.daily[global.daily.len() - 1];
                let yest = &global.daily[global.daily.len() - 2];
                self.trend_turns = today.turn_count - yest.turn_count;
                let yest_total = yest.total_input_tokens
                    + yest.total_cache_read_tokens
                    + yest.total_cache_write_tokens;
                let today_total = today.total_input_tokens
                    + today.total_cache_read_tokens
                    + today.total_cache_write_tokens;
                if yest_total > 0 {
                    let yest_hit = yest.total_cache_read_tokens as f64 / yest_total as f64;
                    let today_hit = today.total_cache_read_tokens as f64 / today_total as f64;
                    self.trend_cache_pct = (today_hit - yest_hit) * 100.0;
                }
                if yest.estimated_cost_usd > 0.0 {
                    self.trend_cost_pct = (today.estimated_cost_usd - yest.estimated_cost_usd)
                        / yest.estimated_cost_usd
                        * 100.0;
                }
            }
        }

        // ── Budget state ──────────────────────────────────────────────────────
        let cost_by_model = db.get_cost_by_model().unwrap_or_default();
        self.budget = build_budget_state(
            &self.global_stats,
            &self.project_stats,
            &cost_by_model,
            &self.config,
            &self.live_stats,
        );

        // ── IS-M: Cache health vital sign ─────────────────────────────────────
        // Detect cache bust by comparing recent vs prior turn efficiency.
        self.budget.cache_bust_drop = self.live_stats.as_ref().and_then(|ls| {
            let sess_id = ls.session.as_ref()?.id.as_str();
            let trend = db.get_cache_efficiency_trend(sess_id, 10).ok()?;
            if trend.len() < 6 {
                return None;
            }
            let recent = &trend[trend.len() - 3..]; // last 3 turns
            let prior = &trend[..trend.len() - 3]; // earlier turns
            let recent_avg = recent.iter().sum::<f64>() / recent.len() as f64;
            let prior_avg = prior.iter().sum::<f64>() / prior.len() as f64;
            if prior_avg > 0.05 && recent_avg < prior_avg * 0.50 {
                Some((1.0 - recent_avg / prior_avg) * 100.0)
            } else {
                None
            }
        });

        // ── IS-E: Bayesian cold-start prior ──────────────────────────────────
        self.budget.median_tokens_per_turn = db.get_mean_tokens_per_turn(90).unwrap_or(50_000.0);

        // ── IS-K: Cost by task type (auto-tagged sessions) ────────────────────
        self.budget.cost_by_tag = db.get_cost_by_tag_days(30).unwrap_or_default();

        // ── C-19: Cost by provider + model ────────────────────────────────────
        // Sort so provider totals are descending, and within each provider models are ordered.
        let raw_pm = db.get_cost_by_provider_and_model().unwrap_or_default();
        // Compute per-provider totals for sort order.
        let mut provider_totals: std::collections::HashMap<&str, f64> =
            std::collections::HashMap::new();
        for (p, _, c) in &raw_pm {
            *provider_totals.entry(p.as_str()).or_default() += c;
        }
        let mut sorted_pm = raw_pm.clone();
        sorted_pm.sort_by(|(pa, ma, ca), (pb, mb, cb)| {
            let ta = provider_totals.get(pa.as_str()).copied().unwrap_or(0.0);
            let tb = provider_totals.get(pb.as_str()).copied().unwrap_or(0.0);
            tb.partial_cmp(&ta)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(cb.partial_cmp(ca).unwrap_or(std::cmp::Ordering::Equal))
                .then(pa.cmp(pb))
                .then(ma.cmp(mb))
        });
        self.budget.cost_by_provider_model = sorted_pm;

        // ── C-17: Build available provider/model lists for scope bar ─────────
        {
            let mut providers: Vec<String> = self
                .sessions_list
                .iter()
                .filter(|s| !s.provider.is_empty())
                .map(|s| s.provider.clone())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            providers.sort();
            self.all_providers = providers;

            // Models for the currently active scope (or all if no scope).
            let scope = self.scope_provider.clone();
            let mut models: Vec<String> = self
                .sessions_list
                .iter()
                .filter(|s| scope.as_deref().map(|p| s.provider == p).unwrap_or(true))
                .filter(|s| !s.model.is_empty())
                .map(|s| s.model.clone())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            models.sort();
            self.all_models = models;

            // Normalize scope: clear stale provider/model selections.
            if let Some(ref sp) = self.scope_provider.clone() {
                if !self.all_providers.contains(sp) {
                    self.scope_provider = None;
                    self.scope_model = None;
                }
            }
            if let Some(ref sm) = self.scope_model.clone() {
                if !self.all_models.contains(sm) {
                    self.scope_model = None;
                }
            }
        }

        // ── Context pressure alerts ───────────────────────────────────────────
        let ctx_pct = self.budget.context_pressure_pct;
        if ctx_pct >= 95.0 && self.context_alert_threshold_crossed < 2 {
            self.context_alert_threshold_crossed = 2;
            self.alert_banner = Some((
                format!(
                    "⚠  Context {:.0}% full — consider /compact or summarizing context",
                    ctx_pct
                ),
                ratatui::style::Color::Red,
            ));
            eprint!("\x07"); // terminal bell
            notify_desktop(
                "Scopeon: Context Critical",
                &format!("Context window {:.0}% full — run /compact now", ctx_pct),
                true,
            );
        } else if ctx_pct >= 80.0 && self.context_alert_threshold_crossed < 1 {
            self.context_alert_threshold_crossed = 1;
            let forecast = self
                .budget
                .predicted_turns_remaining
                .map(|t| format!(" (~{} turns left)", t))
                .unwrap_or_default();
            self.alert_banner = Some((
                format!(
                    "  Context {:.0}% full — approaching limit{}",
                    ctx_pct, forecast
                ),
                ratatui::style::Color::Yellow,
            ));
            eprint!("\x07");
            notify_desktop(
                "Scopeon: Context Warning",
                &format!(
                    "Context window {:.0}% full — approaching limit{}",
                    ctx_pct, forecast
                ),
                false,
            );
        } else if ctx_pct < 70.0 {
            // Pressure dropped (e.g. after /compact) — reset so alerts can fire again
            self.context_alert_threshold_crossed = 0;
            self.alert_banner = None;
        }

        self.last_refresh = Instant::now();
        self.refresh_in_progress = false;
        self.spinner_frame = self.spinner_frame.wrapping_add(1);

        // IS-10: Advance heartbeat pulse phase for animated context bar.
        // Rate scales with context pressure — faster when approaching crisis.
        let pulse_increment = if ctx_pct >= 80.0 {
            0.4
        } else if ctx_pct >= 50.0 {
            0.2
        } else {
            0.08
        };
        self.pulse_phase = (self.pulse_phase + pulse_increment) % 1.0;

        // IS-1: Advance narrative rotation every ~8 refresh cycles.
        self.hint_tick = self.hint_tick.wrapping_add(1);
        if self.hint_tick.is_multiple_of(40) {
            self.narrative_idx = self.narrative_idx.wrapping_add(1);
        }

        // IS-4: Auto-expand zen mode in crisis conditions; auto-re-enter when clear.
        let budget_ratio = if self.budget.daily_limit > 0.0 {
            self.budget.daily_spent / self.budget.daily_limit
        } else {
            0.0
        };
        let is_pressure = ctx_pct >= 80.0 || budget_ratio >= 0.9;
        if self.zen_mode && is_pressure {
            self.zen_mode = false;
            self.zen_auto_exited = true;
            self.zen_clear_cycles = 0;
        } else if self.zen_auto_exited && !self.zen_mode && !is_pressure {
            self.zen_clear_cycles = self.zen_clear_cycles.saturating_add(1);
            if self.zen_clear_cycles >= 3 {
                self.zen_mode = true;
                self.zen_auto_exited = false;
                self.zen_clear_cycles = 0;
                self.toast = Some((
                    "◎ Zen mode restored — pressure cleared".to_string(),
                    std::time::Instant::now(),
                ));
            }
        } else if is_pressure {
            self.zen_clear_cycles = 0;
        }

        // Alert when new unknown models are encountered (pricing accuracy warning).
        if let Ok(mut seen) = scopeon_core::UNKNOWN_MODELS_SEEN.lock() {
            if !seen.is_empty() && self.toast.is_none() {
                let mut names: Vec<&str> = seen.iter().map(|s| s.as_str()).collect();
                names.sort_unstable();
                let models = names.join(", ");
                seen.clear(); // Clear so we don't re-toast on next tick
                self.toast = Some((
                    format!(
                        "⚠ Unknown model(s): {} — using Sonnet pricing (may be inaccurate)",
                        models
                    ),
                    Instant::now(),
                ));
            }
        }

        // Expire toast after 2 seconds
        if let Some((_, ts)) = &self.toast {
            if ts.elapsed() > Duration::from_secs(2) {
                self.toast = None;
            }
        }

        // ── Terminal title update ─────────────────────────────────────────────
        // Writes the xterm OSC sequence to stderr — works while ratatui owns stdout.
        // The title is visible in the OS window switcher and tmux window list,
        // giving ambient awareness without requiring the TUI to be in focus.
        {
            let title = format!(
                "Scopeon ⬡{:.0} | Ctx {:.0}% | ${:.2}",
                self.health_score, ctx_pct, self.budget.daily_spent,
            );
            eprint!("\x1b]0;{}\x07", title);
        }

        // ── Adaptive refresh rate (TRIZ TC-A1: Observability vs. Performance) ──
        // Scale monitoring intensity to risk level. At low context fill, slow
        // refresh saves CPU. In crisis, continuous updates maximise responsiveness.
        self.refresh_interval = if ctx_pct >= 80.0 {
            Duration::from_millis(100) // CRISIS — continuous monitoring
        } else if ctx_pct >= 50.0 {
            Duration::from_millis(500) // ACTIVE — frequent updates
        } else {
            Duration::from_secs(2) // IDLE — low overhead
        };

        // ── Status file IPC (TRIZ D1) ─────────────────────────────────────────
        // Write the shell status one-liner atomically to ~/.cache/scopeon/status.
        // Shell hooks read this file (<1ms, no fork) instead of spawning a subprocess.
        {
            let (h_open, h_close): (&str, &str) = if self.health_score >= 80.0 {
                ("\x1b[92m", "\x1b[0m")
            } else if self.health_score >= 50.0 {
                ("\x1b[93m", "\x1b[0m")
            } else {
                ("\x1b[91m", "\x1b[0m")
            };
            let (c_open, c_close): (&str, &str) = if ctx_pct >= 95.0 {
                ("\x1b[91m", "\x1b[0m")
            } else if ctx_pct >= 80.0 {
                ("\x1b[93m", "\x1b[0m")
            } else {
                ("\x1b[37m", "\x1b[0m")
            };
            let status_line = format!(
                "{h_open}⬡{:.0}{h_close} {c_open}{:.0}%{c_close} \x1b[35m${:.2}\x1b[0m",
                self.health_score, ctx_pct, self.budget.daily_spent,
            );
            let status_path = dirs::cache_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join("scopeon")
                .join("status");
            if let Some(dir) = status_path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = status_path.with_extension("tmp");
            if std::fs::write(&tmp, &status_line).is_ok() {
                let _ = std::fs::rename(&tmp, &status_path);
            }
        }

        // ── Load config providers for Config tab ─────────────────────────────────
        if self.config_providers.is_empty() {
            let reports = list_provider_optimization_reports(&self.config);
            self.config_providers = reports
                .into_iter()
                .map(|r| ConfigProvider {
                    id: r.provider_id.clone(),
                    display_name: r.provider_name.clone(),
                    detected: r.detected,
                    support: r.support.label().to_string(),
                    current_preset: r.current_preset.clone(),
                })
                .collect();
        }
    }

    pub fn handle_key(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        // C-10: Command palette — intercepts all keys when active.
        if self.command_palette_active {
            match key {
                KeyCode::Esc => {
                    self.command_palette_active = false;
                    self.command_palette_query.clear();
                },
                KeyCode::Backspace => {
                    self.command_palette_query.pop();
                },
                KeyCode::Enter => {
                    self.execute_palette_command();
                    self.command_palette_active = false;
                    self.command_palette_query.clear();
                },
                KeyCode::Char(c) => {
                    self.command_palette_query.push(c);
                },
                _ => {},
            }
            return;
        }

        // Help overlay: q/Q quits, Esc just dismisses, any other key dismisses then falls through.
        if self.show_help {
            self.show_help = false;
            match key {
                KeyCode::Char('q') | KeyCode::Char('Q') => {
                    self.quit = true;
                    return;
                },
                KeyCode::Esc => return, // just close
                _ => {},                // dismiss and fall through so the key still takes effect
            }
        }

        // Filter mode in Sessions tab
        if self.sessions_filter_active {
            match key {
                KeyCode::Esc => {
                    self.sessions_filter_active = false;
                    self.sessions_filter.clear();
                    self.sessions_filter_error = None;
                },
                KeyCode::Enter => {
                    self.sessions_filter_active = false;
                    self.sessions_filter_error = None;
                },
                KeyCode::Backspace => {
                    self.sessions_filter.pop();
                    // Re-validate after deletion
                    let f = self.sessions_filter.trim().to_lowercase();
                    self.sessions_filter_error = parse_session_filter(&f).err();
                },
                KeyCode::Char(c) => {
                    self.sessions_filter.push(c);
                    // Validate immediately so error shows while typing
                    let f = self.sessions_filter.trim().to_lowercase();
                    self.sessions_filter_error = parse_session_filter(&f).err();
                },
                _ => {},
            }
            return;
        }

        // Session detail full-screen mode
        if self.session_detail_mode {
            match key {
                // Number keys always escape detail mode and switch tabs (global navigation).
                KeyCode::Char(c @ '1'..='2') => {
                    self.session_detail_mode = false;
                    self.turn_scroll_detail = 0;
                    self.replay_turn_idx = None;
                    let idx = (c as u8 - b'1') as usize;
                    if idx < Tab::count() {
                        self.tab = Tab::from_index(idx);
                        self.pane_focus = PaneFocus::Left;
                    }
                    return;
                },
                KeyCode::Esc => {
                    self.session_detail_mode = false;
                    self.turn_scroll_detail = 0;
                    self.replay_turn_idx = None;
                    self.detail_section = DetailSection::Turns;
                },
                // [ / ] cycle between Turns / Context / MCP & Skills sections
                KeyCode::Char(']') => {
                    self.detail_section = self.detail_section.next();
                    self.turn_scroll_detail = 0;
                },
                KeyCode::Char('[') => {
                    self.detail_section = self.detail_section.prev();
                    self.turn_scroll_detail = 0;
                },
                KeyCode::Down | KeyCode::Char('j') => {
                    self.turn_scroll_detail = self.turn_scroll_detail.saturating_add(1);
                    self.replay_turn_idx = None;
                },
                KeyCode::Up | KeyCode::Char('k') => {
                    self.turn_scroll_detail = self.turn_scroll_detail.saturating_sub(1);
                    self.replay_turn_idx = None;
                },
                KeyCode::Char('g') => {
                    self.turn_scroll_detail = 0;
                    self.replay_turn_idx = None;
                },
                KeyCode::Char('G') => {
                    self.turn_scroll_detail = self
                        .selected_session_stats
                        .as_ref()
                        .map(|s| s.turns.len())
                        .unwrap_or(0);
                    self.replay_turn_idx = None;
                },
                // IS-2: Temporal replay — ← / → scrub through turns.
                KeyCode::Right | KeyCode::Char('l') => {
                    let n_turns = self
                        .selected_session_stats
                        .as_ref()
                        .map(|s| s.turns.len())
                        .unwrap_or(0);
                    if n_turns > 0 {
                        self.replay_turn_idx = Some(match self.replay_turn_idx {
                            None => 0,
                            Some(i) => (i + 1).min(n_turns - 1),
                        });
                    }
                },
                KeyCode::Left | KeyCode::Char('h') => {
                    if let Some(i) = self.replay_turn_idx {
                        if i == 0 {
                            self.replay_turn_idx = None;
                        } else {
                            self.replay_turn_idx = Some(i - 1);
                        }
                    }
                },
                KeyCode::Char('q') => self.quit = true,
                _ => {},
            }
            return;
        }

        // Global keys
        match key {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.quit = true;
                return;
            },
            // C-10: Command palette — Ctrl+P
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.command_palette_active = true;
                self.command_palette_query.clear();
                return;
            },
            KeyCode::Char('?') => {
                self.show_help = true;
                return;
            },
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.last_refresh = Instant::now() - self.refresh_interval;
                self.refresh_in_progress = true;
                return;
            },
            KeyCode::Char('c') => {
                self.copy_stats_to_clipboard();
                return;
            },
            // IS-4: Zen mode toggle — collapse to single-line ambient status.
            KeyCode::Char('z') | KeyCode::Char('Z') => {
                self.zen_mode = !self.zen_mode;
                // Manual toggle overrides auto-re-entry tracking.
                self.zen_auto_exited = false;
                self.zen_clear_cycles = 0;
                return;
            },
            // Provider scope cycling (only on Sessions tab, when ≥2 providers exist).
            KeyCode::Char(']') if self.tab == Tab::Sessions && self.all_providers.len() >= 2 => {
                self.cycle_scope_provider();
                return;
            },
            KeyCode::Char('[') if self.tab == Tab::Sessions && self.all_providers.len() >= 2 => {
                self.cycle_scope_provider_prev();
                return;
            },
            // Model scope cycling — only when a provider is already scoped,
            // so the model row is visible in the scope bar.
            KeyCode::Char('}')
                if self.tab == Tab::Sessions
                    && self.scope_provider.is_some()
                    && !self.all_models.is_empty() =>
            {
                self.cycle_scope_model();
                return;
            },
            KeyCode::Char('{')
                if self.tab == Tab::Sessions
                    && self.scope_provider.is_some()
                    && !self.all_models.is_empty() =>
            {
                self.cycle_scope_model_prev();
                return;
            },
            // Number keys always switch tabs
            KeyCode::Char('1') => {
                self.tab = Tab::Sessions;
                self.pane_focus = PaneFocus::Left;
                return;
            },
            KeyCode::Char('2') => {
                self.tab = Tab::Spend;
                self.pane_focus = PaneFocus::Left;
                return;
            },
            KeyCode::Char('3') => {
                // 3 is no longer a tab — ignore
                return;
            },
            // Tab key: cycle to next tab
            KeyCode::Tab => {
                let next = (self.tab.index() + 1) % Tab::count();
                self.tab = Tab::from_index(next);
                return;
            },
            KeyCode::BackTab => {
                let prev = if self.tab.index() == 0 {
                    Tab::count() - 1
                } else {
                    self.tab.index() - 1
                };
                self.tab = Tab::from_index(prev);
                return;
            },
            _ => {},
        }

        // Tab-specific keys
        match self.tab {
            Tab::Sessions => {
                match key {
                    KeyCode::Down | KeyCode::Char('j') => self.select_session_delta(1),
                    KeyCode::Up | KeyCode::Char('k') => self.select_session_delta(-1),
                    KeyCode::Char('g') => self.select_session_abs(0),
                    KeyCode::Char('G') => self.select_session_abs(usize::MAX),
                    KeyCode::Enter if !self.sessions_list.is_empty() => {
                        self.session_detail_mode = true;
                        self.turn_scroll_detail = 0;
                    },
                    KeyCode::Char('/') => {
                        self.sessions_filter_active = true;
                    },
                    KeyCode::Char('s') => {
                        self.sessions_sort = self.sessions_sort.cycle();
                        self.selected_session_idx = 0;
                        self.selected_session_stats = None;
                    },
                    // Reset scope when Esc is pressed and a scope is active.
                    KeyCode::Esc if self.scope_provider.is_some() || self.scope_model.is_some() => {
                        self.scope_provider = None;
                        self.scope_model = None;
                    },
                    // Toggle Trends chart vs KPI cards
                    KeyCode::Char('t') => {
                        self.show_trends = !self.show_trends;
                    },
                    _ => {},
                }
            },
            Tab::Spend => match key {
                KeyCode::Down | KeyCode::Char('j') => {},
                KeyCode::Up | KeyCode::Char('k') => {},
                _ => {},
            },
            Tab::Config => {
                if self.config_preset_selector_active {
                    // Inside preset selector modal
                    match key {
                        KeyCode::Up | KeyCode::Char('k') => {
                            if self.config_preset_selected_idx > 0 {
                                self.config_preset_selected_idx -= 1;
                            }
                        },
                        KeyCode::Down | KeyCode::Char('j') => {
                            if self.config_preset_selected_idx < 3 {
                                self.config_preset_selected_idx += 1;
                            }
                        },
                        KeyCode::Enter => {
                            // Apply the selected preset
                            self.apply_selected_preset();
                            self.config_preset_selector_active = false;
                        },
                        KeyCode::Esc => {
                            self.config_preset_selector_active = false;
                        },
                        _ => {},
                    }
                } else {
                    // Provider list navigation
                    match key {
                        KeyCode::Up | KeyCode::Char('k') => {
                            if !self.config_providers.is_empty() && self.config_selected_idx > 0 {
                                self.config_selected_idx -= 1;
                            }
                        },
                        KeyCode::Down | KeyCode::Char('j') => {
                            if self.config_selected_idx
                                < self.config_providers.len().saturating_sub(1)
                            {
                                self.config_selected_idx += 1;
                            }
                        },
                        KeyCode::Enter => {
                            // Open preset selector if provider is detected
                            if let Some(provider) =
                                self.config_providers.get(self.config_selected_idx)
                            {
                                if provider.detected {
                                    self.config_preset_selector_active = true;
                                    self.config_preset_selected_idx = 1; // Default to "balanced"
                                }
                            }
                        },
                        _ => {},
                    }
                }
            },
        }
    }

    fn select_session_delta(&mut self, delta: i64) {
        let len = self.filtered_sessions().len();
        if len == 0 {
            return;
        }
        if delta > 0 {
            self.selected_session_idx = (self.selected_session_idx + delta as usize).min(len - 1);
        } else {
            self.selected_session_idx = self.selected_session_idx.saturating_sub((-delta) as usize);
        }
        // Trigger stats reload next refresh by clearing cached session
        self.selected_session_stats = None;
        self.selected_session_tools = None;
    }

    fn select_session_abs(&mut self, idx: usize) {
        let len = self.filtered_sessions().len();
        if len == 0 {
            return;
        }
        self.selected_session_idx = idx.min(len - 1);
        self.selected_session_stats = None;
        self.selected_session_tools = None;
    }

    fn apply_selected_preset(&mut self) {
        let provider = match self.config_providers.get(self.config_selected_idx) {
            Some(p) => p,
            None => return,
        };

        let preset_names = ["most-savings", "balanced", "most-speed", "most-power"];
        let preset_id = match preset_names.get(self.config_preset_selected_idx) {
            Some(name) => match OptimizationPresetId::from_alias(name) {
                Some(id) => id,
                None => return,
            },
            None => return,
        };

        let provider_id = match OptimizationProviderId::from_alias(&provider.id) {
            Some(id) => id,
            None => {
                self.toast = Some((format!("Unknown provider: {}", provider.id), Instant::now()));
                return;
            },
        };

        match apply_provider_preset(provider_id, preset_id, &mut self.config) {
            Ok(_) => {
                self.toast = Some((
                    format!(
                        "Applied {} preset to {}",
                        preset_id.title(),
                        provider.display_name
                    ),
                    Instant::now(),
                ));
                // Reload providers to show updated preset
                self.config_providers.clear();
            },
            Err(e) => {
                self.toast = Some((format!("Failed: {}", e), Instant::now()));
            },
        }
    }

    /// Returns sessions matching the current filter, ordered by `sessions_sort`.
    ///
    /// IS-8: Supports structured natural language predicates in addition to
    /// plain substring matching:
    ///   `cost>5`       — sessions costing more than $5
    ///   `cache<40`     — cache hit rate below 40%
    ///   `tag:feature`  — tagged "feature"
    ///   `today`        — sessions from today
    ///   `anomaly`      — sessions with any anomaly detected
    ///   `model:haiku`  — sessions using a model containing "haiku"
    pub fn filtered_sessions(&self) -> Vec<&Session> {
        let filter = self.sessions_filter.trim().to_lowercase();

        // Parse structured predicates (IS-8). Errors fall through to substring search.
        let structured = parse_session_filter(&filter).ok().flatten();

        let today_str = chrono::Local::now().format("%Y-%m-%d").to_string();

        let mut list: Vec<&Session> = self
            .sessions_list
            .iter()
            .filter(|s| {
                // C-17: Provider/model scope filter — applies first, before text filter.
                if let Some(ref sp) = self.scope_provider {
                    if &s.provider != sp {
                        return false;
                    }
                }
                if let Some(ref sm) = self.scope_model {
                    if &s.model != sm {
                        return false;
                    }
                }

                if filter.is_empty() {
                    return true;
                }

                // Apply structured predicate if parsed.
                if let Some(pred) = &structured {
                    return match pred {
                        SessionFilter::CostGt(threshold) => self
                            .session_summaries
                            .get(&s.id)
                            .map(|ss| ss.estimated_cost_usd > *threshold)
                            .unwrap_or(false),
                        SessionFilter::CostLt(threshold) => self
                            .session_summaries
                            .get(&s.id)
                            .map(|ss| ss.estimated_cost_usd < *threshold)
                            .unwrap_or(false),
                        SessionFilter::CacheGt(pct) => self
                            .session_summaries
                            .get(&s.id)
                            .map(|ss| ss.cache_hit_rate * 100.0 > *pct)
                            .unwrap_or(false),
                        SessionFilter::CacheLt(pct) => self
                            .session_summaries
                            .get(&s.id)
                            .map(|ss| ss.cache_hit_rate * 100.0 < *pct)
                            .unwrap_or(false),
                        SessionFilter::Tag(tag) => {
                            s.id.to_lowercase().contains(tag.as_str())
                                || s.project_name.to_lowercase().contains(tag.as_str())
                        },
                        SessionFilter::Today => {
                            let session_ms = s.last_turn_at;
                            let session_date = chrono::DateTime::from_timestamp_millis(session_ms)
                                .map(|dt| {
                                    dt.with_timezone(&chrono::Local)
                                        .format("%Y-%m-%d")
                                        .to_string()
                                })
                                .unwrap_or_default();
                            session_date == today_str
                        },
                        SessionFilter::Anomaly => {
                            self.session_anomalies.iter().any(|a| a.session_id == s.id)
                        },
                        SessionFilter::Model(m) => s.model.to_lowercase().contains(m.as_str()),
                    };
                }

                // Plain substring fallback.
                s.project_name.to_lowercase().contains(&filter)
                    || s.git_branch.to_lowercase().contains(&filter)
                    || s.model.to_lowercase().contains(&filter)
            })
            .collect();

        match self.sessions_sort {
            SessionSort::Newest => {}, // sessions_list is already newest-first from DB
            SessionSort::Oldest => list.reverse(),
            SessionSort::MostExpensive => {
                list.sort_by(|a, b| {
                    let cost_a = self
                        .session_summaries
                        .get(&a.id)
                        .map(|s| s.estimated_cost_usd)
                        .unwrap_or(0.0);
                    let cost_b = self
                        .session_summaries
                        .get(&b.id)
                        .map(|s| s.estimated_cost_usd)
                        .unwrap_or(0.0);
                    cost_b
                        .partial_cmp(&cost_a)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            },
        }

        list
    }

    // C-17: Cycle through available providers (None → first → … → last → None).
    fn cycle_scope_provider(&mut self) {
        self.scope_model = None; // reset model when provider changes
        if self.all_providers.is_empty() {
            return;
        }
        self.scope_provider = match self.scope_provider.take() {
            None => Some(self.all_providers[0].clone()),
            Some(ref p) => {
                let idx = self.all_providers.iter().position(|x| x == p);
                match idx {
                    Some(i) if i + 1 < self.all_providers.len() => {
                        Some(self.all_providers[i + 1].clone())
                    },
                    _ => None,
                }
            },
        };
    }

    // Cycle through available models for the current scope.
    fn cycle_scope_model(&mut self) {
        if self.all_models.is_empty() {
            return;
        }
        self.scope_model = match self.scope_model.take() {
            None => Some(self.all_models[0].clone()),
            Some(ref m) => {
                let idx = self.all_models.iter().position(|x| x == m);
                match idx {
                    Some(i) if i + 1 < self.all_models.len() => {
                        Some(self.all_models[i + 1].clone())
                    },
                    _ => None,
                }
            },
        };
    }

    fn cycle_scope_provider_prev(&mut self) {
        self.scope_model = None;
        if self.all_providers.is_empty() {
            return;
        }
        self.scope_provider = match self.scope_provider.take() {
            None => Some(self.all_providers[self.all_providers.len() - 1].clone()),
            Some(ref p) => {
                let idx = self.all_providers.iter().position(|x| x == p);
                match idx {
                    Some(0) => None,
                    Some(i) => Some(self.all_providers[i - 1].clone()),
                    None => None,
                }
            },
        };
    }

    fn cycle_scope_model_prev(&mut self) {
        if self.all_models.is_empty() {
            return;
        }
        self.scope_model = match self.scope_model.take() {
            None => Some(self.all_models[self.all_models.len() - 1].clone()),
            Some(ref m) => {
                let idx = self.all_models.iter().position(|x| x == m);
                match idx {
                    Some(0) => None,
                    Some(i) => Some(self.all_models[i - 1].clone()),
                    None => None,
                }
            },
        };
    }

    // C-10: Execute the palette command matching the current query.
    fn execute_palette_command(&mut self) {
        let q = self.command_palette_query.trim().to_lowercase();
        let items = Self::palette_items();
        if let Some(item) = items
            .iter()
            .find(|(label, _, _)| label.to_lowercase().contains(&q))
        {
            (item.1)(self);
        }
    }

    /// Returns (label, action_fn, description) for all command palette items.
    /// Used both for rendering and execution.
    #[allow(clippy::type_complexity)]
    pub fn palette_items() -> Vec<(&'static str, fn(&mut App), &'static str)> {
        vec![
            (
                "1 Sessions",
                |a| {
                    a.tab = Tab::Sessions;
                },
                "Go to Sessions tab",
            ),
            (
                "2 Spend",
                |a| {
                    a.tab = Tab::Spend;
                },
                "Go to Spend tab",
            ),
            (
                "refresh",
                |a| {
                    a.last_refresh = Instant::now() - a.refresh_interval;
                    a.refresh_in_progress = true;
                },
                "Force data refresh",
            ),
            (
                "zen",
                |a| {
                    a.zen_mode = !a.zen_mode;
                    a.zen_auto_exited = false;
                },
                "Toggle zen mode",
            ),
            (
                "filter",
                |a| {
                    a.tab = Tab::Sessions;
                    a.sessions_filter_active = true;
                    a.sessions_filter.clear();
                },
                "Open session filter",
            ),
            (
                "copy stats",
                |a| a.copy_stats_to_clipboard(),
                "Copy stats to clipboard",
            ),
            (
                "theme cockpit",
                |a| {
                    a.theme = Theme::Cockpit;
                },
                "Switch to Cockpit theme",
            ),
            (
                "theme standard",
                |a| {
                    a.theme = Theme::Standard;
                },
                "Switch to Standard theme",
            ),
            (
                "theme contrast",
                |a| {
                    a.theme = Theme::HighContrast;
                },
                "Switch to High Contrast theme",
            ),
            (
                "help",
                |a| {
                    a.show_help = true;
                },
                "Show help overlay",
            ),
        ]
    }

    /// Copy a formatted summary of current stats to the system clipboard.
    /// Shows a toast notification in the status bar for 2 seconds.
    pub fn copy_stats_to_clipboard(&mut self) {
        let text = self.format_clipboard_summary();
        let result = arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text));
        match result {
            Ok(_) => {
                self.toast = Some(("Stats copied to clipboard".to_string(), Instant::now()));
            },
            Err(_) => {
                self.toast = Some((
                    "Copy failed — no clipboard available".to_string(),
                    Instant::now(),
                ));
            },
        }
    }

    fn format_clipboard_summary(&self) -> String {
        let mut out = String::new();

        out.push_str(&format!(
            "# Scopeon Stats — {}\n\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M")
        ));

        // Health
        out.push_str(&format!("**Health Score:** {:.0}/100\n", self.health_score));

        // Daily spend
        out.push_str(&format!(
            "**Daily Cost:** ${:.4}\n",
            self.budget.daily_spent
        ));

        // Cache
        if let Some(stats) = &self.live_stats {
            out.push_str(&format!(
                "**Cache Hit Rate:** {:.1}%\n",
                stats.cache_hit_rate * 100.0
            ));
            out.push_str(&format!("**Total Turns:** {}\n", stats.total_turns));
            out.push_str(&format!(
                "**Est. Cost:** ${:.4}\n",
                stats.estimated_cost_usd
            ));
            out.push_str(&format!(
                "**Cache Savings:** ${:.4}\n",
                stats.cache_savings_usd
            ));
            if let Some(session) = &stats.session {
                out.push_str(&format!("**Project:** {}\n", session.project_name));
                out.push_str(&format!("**Branch:** {}\n", session.git_branch));
                out.push_str(&format!("**Model:** {}\n", session.model));
            }
        }

        // Context
        if self.budget.context_pressure_pct > 0.0 {
            out.push_str(&format!(
                "**Context Usage:** {:.0}%\n",
                self.budget.context_pressure_pct
            ));
        }

        out.push_str("\n---\n");
        out.push_str(&format!(
            "Generated by Scopeon v{}\n",
            env!("CARGO_PKG_VERSION")
        ));

        out
    }

    /// Handle a mouse event from the terminal.
    pub fn handle_mouse(&mut self, column: u16, row: u16, kind: MouseEventKind) {
        match kind {
            // Tab bar click (row 0)
            MouseEventKind::Down(MouseButton::Left) if row == 0 => {
                if let Some(tab) = tab_at_x(column, self.tab) {
                    self.tab = tab;
                    self.pane_focus = PaneFocus::Left;
                    self.replay_turn_idx = None;
                }
            },
            // IS-5: Session list row click — select session; second click on same row = open detail.
            MouseEventKind::Down(MouseButton::Left)
                if self.tab == Tab::Sessions && !self.session_detail_mode =>
            {
                // Gate clicks to the left (session list) panel.
                // When the terminal is wide enough for split-panel, right panel
                // starts at ~38% of the terminal width.
                let split_threshold = 120u16;
                let list_panel_w = if self.terminal_width >= split_threshold {
                    (self.terminal_width as f32 * 0.38) as u16
                } else {
                    self.terminal_width
                };
                if column >= list_panel_w {
                    // Click landed in the detail preview panel — ignore for selection.
                } else {
                    // Compute where the session list body starts:
                    //   row 0       = tab bar
                    //   row 1       = alert banner (0 or 1 line)
                    //   rows 2..N   = overview cards (7 lines when data exists and terminal is tall)
                    //   row N+1     = session list top border
                    //   row N+2     = first session content row
                    let banner_h = if self.alert_banner.is_some() {
                        1u16
                    } else {
                        0u16
                    };
                    let has_data = self.budget.daily_spent > 0.0 || self.global_stats.is_some();
                    // Derive scope_h and cards_h using the same logic as sessions::draw().
                    let scope_h = crate::views::sessions::compute_scope_h(self);
                    let content_h = self.terminal_height.saturating_sub(2 + banner_h);
                    let cards_h = if has_data && content_h >= 14 + scope_h {
                        7u16
                    } else {
                        0u16
                    };
                    let body_start = 1u16 + banner_h + cards_h + scope_h + 1u16;
                    if row >= body_start {
                        let row_in_body = row - body_start;
                        let row_h = 2u16; // each session row renders project + cost (2 lines)
                        let visual_idx = (row_in_body / row_h) as usize;

                        // Recompute the same scroll offset used by draw_session_list.
                        let visible_height =
                            self.terminal_height.saturating_sub(body_start + 2) as usize;
                        let visible_height = visible_height.max(4);
                        let scroll = if self.selected_session_idx >= visible_height {
                            self.selected_session_idx - visible_height + 1
                        } else {
                            0
                        };
                        let new_idx = scroll + visual_idx;
                        let sessions = self.filtered_sessions();
                        if new_idx < sessions.len() {
                            if self.mouse_last_click_row == Some(row)
                                && self.selected_session_idx == new_idx
                            {
                                // Double-click same row → open detail
                                self.session_detail_mode = true;
                                self.turn_scroll_detail = 0;
                                self.replay_turn_idx = None;
                            } else {
                                self.select_session_abs(new_idx);
                            }
                            self.mouse_last_click_row = Some(row);
                        }
                    }
                } // end left-panel gate
            },
            // Scroll wheel — delegate to view-specific scroll
            MouseEventKind::ScrollUp => self.scroll_up(),
            MouseEventKind::ScrollDown => self.scroll_down(),
            _ => {},
        }
    }

    fn scroll_up(&mut self) {
        match self.tab {
            Tab::Sessions if self.session_detail_mode => {
                self.turn_scroll_detail = self.turn_scroll_detail.saturating_sub(1);
            },
            Tab::Sessions => {
                self.select_session_delta(-1);
            },
            _ => {},
        }
    }

    fn scroll_down(&mut self) {
        match self.tab {
            Tab::Sessions if self.session_detail_mode => {
                self.turn_scroll_detail = self.turn_scroll_detail.saturating_add(1);
            },
            Tab::Sessions => {
                self.select_session_delta(1);
            },
            _ => {},
        }
    }
} // end impl App

/// Map an x-coordinate click on the tab bar (row 0) to a Tab.
///
/// Tab bar layout:
///   " ◈ Scopeon  " (12 chars)  then for each tab: " ┃ " (3) + label (varies)
fn tab_at_x(x: u16, active: Tab) -> Option<Tab> {
    let badge_w = 12u16; // " ◈ Scopeon  "
    let sep_w = 3u16; // " ┃ "

    let tabs: &[(&str, Tab, &str)] =
        &[("1", Tab::Sessions, "Sessions"), ("2", Tab::Spend, "Spend")];

    let mut cur_x = badge_w;
    for (key, tab, label) in tabs {
        let tab_w = if *tab == active {
            (label.len() + 4) as u16 // " N◆Label "
        } else {
            (key.len() + 1 + label.len()) as u16 // "N:Label"
        };
        let start = cur_x + sep_w;
        let end = start + tab_w;
        if x >= start && x < end {
            return Some(*tab);
        }
        cur_x = end;
    }
    None
}

/// IS-8: Structured session filter predicates parsed from the `/` search box.
#[derive(Debug)]
enum SessionFilter {
    CostGt(f64),
    CostLt(f64),
    CacheGt(f64),
    CacheLt(f64),
    Tag(String),
    Today,
    Anomaly,
    Model(String),
}

/// IS-8: Parse a filter string into a structured predicate, or return None for
/// plain substring mode. Syntax:
///   cost>5   cost<5   cache>70   cache<40
///   tag:feature   model:haiku   today   anomaly
/// Returns `Ok(Some(filter))` for a recognized structured filter,
/// `Ok(None)` for plain substring search, or `Err(msg)` for a parse error
/// (e.g., `cost>abc` where "abc" is not a number).
fn parse_session_filter(filter: &str) -> Result<Option<SessionFilter>, String> {
    let f = filter.trim();
    if f == "today" {
        return Ok(Some(SessionFilter::Today));
    }
    if f == "anomaly" {
        return Ok(Some(SessionFilter::Anomaly));
    }
    if let Some(rest) = f.strip_prefix("tag:") {
        return Ok(Some(SessionFilter::Tag(rest.to_string())));
    }
    if let Some(rest) = f.strip_prefix("model:") {
        return Ok(Some(SessionFilter::Model(rest.to_string())));
    }
    if let Some(rest) = f.strip_prefix("cost>") {
        return rest
            .parse::<f64>()
            .map(|v| Some(SessionFilter::CostGt(v)))
            .map_err(|_| format!("cost>{} │ ✗ expected a number", rest));
    }
    if let Some(rest) = f.strip_prefix("cost<") {
        return rest
            .parse::<f64>()
            .map(|v| Some(SessionFilter::CostLt(v)))
            .map_err(|_| format!("cost<{} │ ✗ expected a number", rest));
    }
    if let Some(rest) = f.strip_prefix("cache>") {
        return rest
            .parse::<f64>()
            .map(|v| Some(SessionFilter::CacheGt(v)))
            .map_err(|_| format!("cache>{} │ ✗ expected a number", rest));
    }
    if let Some(rest) = f.strip_prefix("cache<") {
        return rest
            .parse::<f64>()
            .map(|v| Some(SessionFilter::CacheLt(v)))
            .map_err(|_| format!("cache<{} │ ✗ expected a number", rest));
    }
    Ok(None)
}

fn detect_copilot_activity() -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    let sessions_dir = home.join(".copilot/session-state");
    if !sessions_dir.exists() {
        return false;
    }

    let threshold = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(5 * 60))
        .unwrap_or(std::time::UNIX_EPOCH);

    // Check both flat *.jsonl files and session dir events.jsonl for freshness
    let Ok(entries) = std::fs::read_dir(&sessions_dir) else {
        return false;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        // Flat JSONL session file
        if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(modified) = meta.modified() {
                    if modified > threshold {
                        return true;
                    }
                }
            }
        }
        // Session directory with events.jsonl
        if path.is_dir() {
            let events = path.join("events.jsonl");
            if let Ok(meta) = std::fs::metadata(&events) {
                if let Ok(modified) = meta.modified() {
                    if modified > threshold {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn build_provider_status(
    provider_db_stats: &std::collections::HashMap<String, (i64, i64)>,
    config: &UserConfig,
) -> Vec<ProviderStatus> {
    let home = dirs::home_dir();
    let optimization_reports: std::collections::HashMap<_, _> =
        list_provider_optimization_reports(config)
            .into_iter()
            .map(|report| (report.provider_id.clone(), report))
            .collect();

    let check =
        |sub: &str| -> bool { home.as_ref().map(|h| h.join(sub).exists()).unwrap_or(false) };
    let check_abs = |path: &str| -> bool { std::path::Path::new(path).exists() };

    // Respect CLAUDE_CONFIG_DIR override (same priority as ClaudeCodeProvider::new).
    let claude_available = std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".claude")))
        .unwrap_or_else(|| std::path::PathBuf::from("/nonexistent"))
        .join("projects")
        .exists();

    let ollama_available = check("Library/Application Support/Ollama/db.sqlite");
    let codex_available = check(".codex/sessions");
    let aider_available = check(".aider/analytics.jsonl");
    let gemini_available = check(".gemini/tmp");
    let copilot_cli_available = check(".copilot/session-state");
    let cursor_available = check_abs("/Applications/Cursor.app")
        || check("Library/Application Support/Cursor/User/globalStorage")
        || check(".config/Cursor/User/globalStorage");
    let windsurf_available = check_abs("/Applications/Windsurf.app")
        || check("Library/Application Support/Windsurf/User/globalStorage");
    let continue_available =
        check("Library/Application Support/Code/User/globalStorage/continue.continue")
            || check(".config/Code/User/globalStorage/continue.continue");

    let db_stats = |id: &str| -> (usize, usize) {
        provider_db_stats
            .get(id)
            .map(|&(s, t)| (s as usize, t as usize))
            .unwrap_or((0, 0))
    };
    let optimization_status = |id: &str| -> String {
        optimization_reports
            .get(id)
            .map(|report| {
                let preset = report
                    .current_preset
                    .as_deref()
                    .map(|value| format!("preset {value}"))
                    .unwrap_or_else(|| "preset not applied".to_string());
                format!("Optimize: {} · {}", report.support.label(), preset)
            })
            .unwrap_or_else(|| "Optimize: observe only".to_string())
    };

    let (claude_sessions, claude_turns) = db_stats("claude-code");
    let (copilot_sessions, copilot_turns) = db_stats("copilot-cli");
    let (aider_sessions, aider_turns) = db_stats("aider");
    let (gemini_sessions, gemini_turns) = db_stats("gemini-cli");
    let (ollama_sessions, ollama_turns) = db_stats("ollama");
    let (codex_sessions, codex_turns) = db_stats("codex");

    vec![
        ProviderStatus {
            id: "claude-code".to_string(),
            name: "Claude Code".to_string(),
            is_active: claude_available,
            session_count: claude_sessions,
            turn_count: claude_turns,
            last_update: None,
            config_hint: "~/.claude/projects/  (auto-detected)".to_string(),
            optimization_status: optimization_status("claude-code"),
        },
        ProviderStatus {
            id: "copilot-cli".to_string(),
            name: "GitHub Copilot CLI".to_string(),
            is_active: copilot_cli_available,
            session_count: copilot_sessions,
            turn_count: copilot_turns,
            last_update: None,
            config_hint: if copilot_cli_available {
                "~/.copilot/session-state/  (full JSONL data — context, turns, tools, compaction tokens)".to_string()
            } else {
                "Install GitHub Copilot CLI from github.com/github/gh-copilot".to_string()
            },
            optimization_status: optimization_status("copilot-cli"),
        },
        ProviderStatus {
            id: "aider".to_string(),
            name: "Aider".to_string(),
            is_active: aider_available,
            session_count: aider_sessions,
            turn_count: aider_turns,
            last_update: None,
            config_hint: if aider_available {
                "~/.aider/analytics.jsonl  (run aider with --analytics-log)".to_string()
            } else {
                "Enable: run `aider --analytics` or set AIDER_ANALYTICS_LOG".to_string()
            },
            optimization_status: optimization_status("aider"),
        },
        ProviderStatus {
            id: "gemini-cli".to_string(),
            name: "Gemini CLI".to_string(),
            is_active: gemini_available,
            session_count: gemini_sessions,
            turn_count: gemini_turns,
            last_update: None,
            config_hint: if gemini_available {
                "~/.gemini/tmp/*/session-*.jsonl  (auto-detected)".to_string()
            } else {
                "Install: npm install -g @google/generative-ai-cli".to_string()
            },
            optimization_status: optimization_status("gemini-cli"),
        },
        ProviderStatus {
            id: "ollama".to_string(),
            name: "Ollama (local LLM)".to_string(),
            is_active: ollama_available,
            session_count: ollama_sessions,
            turn_count: ollama_turns,
            last_update: None,
            config_hint: "~/Library/Application Support/Ollama/db.sqlite".to_string(),
            optimization_status: optimization_status("ollama"),
        },
        ProviderStatus {
            id: "codex".to_string(),
            name: "OpenAI Codex CLI".to_string(),
            is_active: codex_available,
            session_count: codex_sessions,
            turn_count: codex_turns,
            last_update: None,
            config_hint: if codex_available {
                "~/.codex/sessions/YYYY/MM/DD/*.jsonl  (auto-detected)".to_string()
            } else {
                "Install: npm install -g @openai/codex".to_string()
            },
            optimization_status: optimization_status("codex"),
        },
        ProviderStatus {
            id: "cursor".to_string(),
            name: "Cursor".to_string(),
            is_active: cursor_available,
            session_count: 0,
            turn_count: 0,
            last_update: None,
            config_hint: if cursor_available {
                "Detected — token data in binary LevelDB (not yet accessible)".to_string()
            } else {
                "Install from: cursor.com".to_string()
            },
            optimization_status: optimization_status("cursor"),
        },
        ProviderStatus {
            id: "windsurf".to_string(),
            name: "Windsurf".to_string(),
            is_active: windsurf_available,
            session_count: 0,
            turn_count: 0,
            last_update: None,
            config_hint: if windsurf_available {
                "Detected — token data in binary LevelDB (not yet accessible)".to_string()
            } else {
                "Install from: codeium.com/windsurf".to_string()
            },
            optimization_status: optimization_status("windsurf"),
        },
        ProviderStatus {
            id: "continue".to_string(),
            name: "Continue.dev".to_string(),
            is_active: continue_available,
            session_count: 0,
            turn_count: 0,
            last_update: None,
            config_hint: if continue_available {
                "Detected — use Continue's analytics export to enable tracking".to_string()
            } else {
                "Install Continue extension in VS Code: continue.dev".to_string()
            },
            optimization_status: optimization_status("continue"),
        },
    ]
}

fn build_budget_state(
    global: &Option<GlobalStats>,
    project_stats: &[scopeon_core::ProjectStats],
    cost_by_model: &[(String, f64)],
    config: &UserConfig,
    live_stats: &Option<SessionStats>,
) -> BudgetState {
    let Some(global) = global else {
        return BudgetState::default();
    };

    let today = chrono::Local::now().date_naive();
    let week_start = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);

    let mut daily_spent = 0.0f64;
    let mut weekly_spent = 0.0f64;
    let mut monthly_spent = 0.0f64;
    let mut daily_history: Vec<(String, f64)> = Vec::new();

    for r in &global.daily {
        if let Ok(date) = chrono::NaiveDate::parse_from_str(&r.date, "%Y-%m-%d") {
            if date == today {
                daily_spent += r.estimated_cost_usd;
            }
            if date >= week_start {
                weekly_spent += r.estimated_cost_usd;
            }
            if date.month() == today.month() && date.year() == today.year() {
                monthly_spent += r.estimated_cost_usd;
            }
        }
        daily_history.push((r.date.clone(), r.estimated_cost_usd));
    }
    daily_history.sort_by(|a, b| b.0.cmp(&a.0));
    daily_history.truncate(14);

    // Projection: avg daily cost × 30
    let avg_daily = if !global.daily.is_empty() {
        global.estimated_cost_usd / global.daily.len() as f64
    } else {
        0.0
    };
    let projected_monthly = avg_daily * 30.0;

    // Cost by model: approximate from session model names via project_stats
    // We use project_stats.total_cost_usd per project as a proxy
    let cost_by_project: Vec<(String, f64)> = project_stats
        .iter()
        .map(|p| (p.project_name.clone(), p.total_cost_usd))
        .collect();

    // Context pressure from live session's last turn
    let (context_pressure_pct, context_tokens_remaining, context_window, predicted_turns_remaining) =
        live_stats
            .as_ref()
            .and_then(|s| {
                let session = s.session.as_ref()?;
                let model = session.model.as_str();
                let stored_window = session.context_window_tokens;
                let last = s.turns.last()?;
                let input = last.input_tokens + last.cache_read_tokens + last.cache_write_tokens;
                // §8.2: use stored context window from JSONL if available
                let window =
                    stored_window.unwrap_or_else(|| scopeon_core::context_window_for_model(model));
                let (pct, remaining) =
                    scopeon_core::context_pressure_with_window(model, input, stored_window);
                // IS-E: use Bayesian prior (50k median) for cold-start (< 3 turns);
                // blend toward data-driven prediction as turns accumulate.
                let predicted = predict_turns_remaining_bayesian(&s.turns, remaining, 50_000.0);
                Some((pct, remaining, window, predicted))
            })
            .unwrap_or((0.0, 0, 128_000, None));

    BudgetState {
        daily_spent,
        weekly_spent,
        monthly_spent,
        daily_limit: config.budget.daily_usd,
        weekly_limit: config.budget.weekly_usd,
        monthly_limit: config.budget.monthly_usd,
        projected_monthly,
        cost_by_model: cost_by_model.to_vec(),
        cost_by_project,
        daily_history: daily_history.clone(),
        context_pressure_pct,
        context_tokens_remaining,
        context_window,
        predicted_turns_remaining,
        predicted_days_until_monthly_limit: predict_days_until_monthly_limit(
            &daily_history,
            config.budget.monthly_usd,
            monthly_spent,
        ),
        // These are populated post-construction in refresh() where DB access is available.
        cache_bust_drop: None,
        median_tokens_per_turn: 50_000.0,
        cost_by_tag: Vec::new(),
        cost_by_provider_model: Vec::new(), // populated post-construction in refresh()
        // IS-14: EOD spend projection based on hours elapsed today.
        daily_hourly_rate: compute_daily_hourly_rate(daily_spent),
        daily_projected_eod: compute_daily_projected_eod(daily_spent),
    }
}

/// IS-14: Compute the current hourly spending rate based on hours elapsed today.
fn compute_daily_hourly_rate(daily_spent: f64) -> f64 {
    let now = chrono::Local::now();
    let hours_elapsed =
        now.hour() as f64 + now.minute() as f64 / 60.0 + now.second() as f64 / 3600.0;
    if hours_elapsed < 0.1 {
        0.0
    } else {
        daily_spent / hours_elapsed
    }
}

/// IS-14: Project end-of-day spend by extrapolating current hourly rate.
fn compute_daily_projected_eod(daily_spent: f64) -> f64 {
    let now = chrono::Local::now();
    let hours_elapsed =
        now.hour() as f64 + now.minute() as f64 / 60.0 + now.second() as f64 / 3600.0;
    if hours_elapsed < 0.5 {
        // Too early in the day — projection unreliable
        0.0
    } else {
        let rate = daily_spent / hours_elapsed;
        rate * 24.0
    }
}

/// Fit a least-squares linear slope to the last ≤10 turns' token sizes and
/// return how many more turns are predicted before `tokens_remaining` runs out.
///
/// Returns `None` when there are fewer than 3 data points, or when the trend
/// is flat / decreasing (no meaningful countdown to show).
fn predict_turns_remaining_from_turns(
    turns: &[scopeon_core::Turn],
    tokens_remaining: i64,
) -> Option<i64> {
    let window: Vec<_> = turns.iter().rev().take(10).rev().collect();
    if window.len() < 3 {
        return None;
    }
    let n = window.len() as f64;
    let ys: Vec<f64> = window
        .iter()
        .map(|t| (t.input_tokens + t.cache_read_tokens + t.cache_write_tokens) as f64)
        .collect();
    let x_mean = (n - 1.0) / 2.0;
    let y_mean = ys.iter().sum::<f64>() / n;
    let numerator: f64 = ys
        .iter()
        .enumerate()
        .map(|(i, &y)| (i as f64 - x_mean) * (y - y_mean))
        .sum();
    let denominator: f64 = (0..window.len()).map(|i| (i as f64 - x_mean).powi(2)).sum();
    if denominator < 1.0 {
        return None;
    }
    let slope = numerator / denominator;
    if slope <= 0.0 {
        return None;
    }
    Some((tokens_remaining as f64 / slope).round().max(0.0) as i64)
}

/// IS-E: Bayesian cold-start countdown — blends a Bayesian prior with the
/// data-driven regression estimate, allowing a useful forecast from turn zero.
///
/// The prior weight decreases linearly from 100% at turn 0 to 0% at turn 10+,
/// after which the purely data-driven estimate takes over.
fn predict_turns_remaining_bayesian(
    turns: &[scopeon_core::Turn],
    tokens_remaining: i64,
    prior_median_tokens: f64,
) -> Option<i64> {
    let n = turns.len();
    // Prior estimate: tokens_remaining / median tokens per turn
    let prior = if prior_median_tokens > 0.0 {
        (tokens_remaining as f64 / prior_median_tokens)
            .round()
            .max(0.0) as i64
    } else {
        return None;
    };

    if n < 3 {
        // Pure prior for early turns
        return Some(prior);
    }

    // Data-driven estimate
    let data_est = predict_turns_remaining_from_turns(turns, tokens_remaining);
    let data_est = match data_est {
        Some(d) => d,
        None => return Some(prior), // regression not available, use prior
    };

    // Blend weight: prior weight decreases from 1.0 at n=3 to 0.0 at n≥13
    let prior_weight = (1.0 - (n.saturating_sub(3) as f64 / 10.0)).max(0.0);
    let blended =
        (prior as f64 * prior_weight + data_est as f64 * (1.0 - prior_weight)).round() as i64;
    Some(blended.max(0))
}

/// Forecast how many days until the monthly budget limit is exhausted using
/// linear regression on the last 7 days of daily costs (newest-first slice).
///
/// TRIZ D3: Resolves NE-D (no budget exhaustion forecast). Uses the same
/// least-squares technique as `predict_turns_remaining_from_turns`.
///
/// Returns `None` when:
/// - No monthly limit is configured (`limit_usd == 0.0`)
/// - Fewer than 3 daily history entries exist
/// - Spending slope is flat or declining (no forecast possible)
fn predict_days_until_monthly_limit(
    daily_history: &[(String, f64)], // newest-first
    limit_usd: f64,
    monthly_spent: f64,
) -> Option<f64> {
    if limit_usd <= 0.0 {
        return None;
    }
    let remaining = limit_usd - monthly_spent;
    if remaining <= 0.0 {
        return None; // already over budget
    }

    // Take up to 7 entries, reverse so index 0 = oldest.
    let window: Vec<f64> = daily_history
        .iter()
        .take(7)
        .map(|(_, c)| *c)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    if window.len() < 3 {
        return None;
    }

    let n = window.len() as f64;
    let x_mean = (n - 1.0) / 2.0;
    let y_mean = window.iter().sum::<f64>() / n;
    let numerator: f64 = window
        .iter()
        .enumerate()
        .map(|(i, &y)| (i as f64 - x_mean) * (y - y_mean))
        .sum();
    let denominator: f64 = (0..window.len()).map(|i| (i as f64 - x_mean).powi(2)).sum();
    if denominator < 1e-10 {
        return None;
    }
    let slope = numerator / denominator; // $/day trend
    if slope <= 0.0 {
        return None; // spending not growing
    }

    Some((remaining / slope).max(0.0))
}

/// Fire a non-blocking OS desktop notification for context pressure thresholds.
///
/// Uses `osascript` on macOS and `notify-send` on Linux. Silently no-ops on
/// other platforms or if the notification binary is unavailable.
fn notify_desktop(title: &str, body: &str, urgent: bool) {
    // Sanitise: replace double-quotes to avoid injection in osascript.
    let title = title.replace('"', "'");
    let body = body.replace('"', "'");

    #[cfg(target_os = "macos")]
    {
        let _ = urgent; // used on Linux only
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!(
                r#"display notification "{}" with title "{}""#,
                body, title
            ))
            .spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let urgency = if urgent { "critical" } else { "normal" };
        let _ = std::process::Command::new("notify-send")
            .args(["--urgency", urgency, "--app-name", "Scopeon"])
            .arg(&title)
            .arg(&body)
            .spawn();
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = (title, body, urgent);
}

pub async fn run_tui(db: Arc<Mutex<Database>>) -> Result<()> {
    // Open a read-only replica for the TUI refresh path.
    // Under WAL mode, readers never block writers and vice versa, so the TUI
    // can refresh without ever contending with the file-watcher write mutex.
    let db_ro: Option<Database> = db
        .lock()
        .ok()
        .and_then(|g| g.path().map(|p| p.to_owned()))
        .and_then(|p| Database::open_readonly(&p).ok());

    enable_raw_mode()?;

    // Queue all terminal-setup commands into one write+flush so the terminal
    // emulator sees EnterAlternateScreen and Clear as a single atomic chunk.
    // This eliminates the flash of old alternate-screen content that appears
    // between EnterAlternateScreen and a subsequent separate clear call.
    {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        stdout.queue(EnterAlternateScreen)?;
        stdout.queue(TermClear(ClearType::All))?;
        stdout.queue(cursor::MoveTo(0, 0))?;
        stdout.queue(EnableFocusChange)?;
        stdout.queue(EnableMouseCapture)?;
        stdout.flush()?;
    }

    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    // ── Splash screen ─────────────────────────────────────────────────────────
    // Draw the branded splash while the initial DB refresh runs.
    terminal.draw(|f| crate::ui::draw_splash(f, app.theme))?;
    let splash_start = Instant::now();

    app.refresh_in_progress = true;
    if let Some(ro) = &db_ro {
        app.refresh(ro);
    } else if let Ok(db_guard) = db.try_lock() {
        app.refresh(&db_guard);
    }
    // If try_lock fails the backfill holds the mutex; spinner stays visible
    // and we retry after the splash delay.

    // Hold splash for at least 1 500 ms so it's actually readable.
    let elapsed = splash_start.elapsed();
    if elapsed < Duration::from_millis(1500) {
        std::thread::sleep(Duration::from_millis(1500) - elapsed);
    }

    loop {
        if let Ok(size) = terminal.size() {
            app.terminal_height = size.height;
            app.terminal_width = size.width;
        }
        terminal.draw(|f| draw(f, &app))?;

        // Increment hint rotation ticker every ~8 render cycles (~1.6s at 200ms)
        app.hint_tick = app.hint_tick.wrapping_add(1);
        // Advance spinner frame every render tick
        if app.refresh_in_progress {
            app.spinner_frame = app.spinner_frame.wrapping_add(1);
        }

        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) => {
                    app.handle_key(key.code, key.modifiers);
                    if app.quit {
                        break;
                    }
                },
                // Mouse events — tab switching and scroll wheel
                Event::Mouse(mouse) => {
                    app.handle_mouse(mouse.column, mouse.row, mouse.kind);
                },
                // Show "too small" screen and clear on resize.
                Event::Resize(_, _) => {
                    terminal.clear()?;
                },
                // Clear on focus regain — covers cases where another window
                // was overlaid and the terminal compositor didn't fully restore.
                Event::FocusGained => {
                    terminal.clear()?;
                },
                _ => {},
            }
        }

        if app.last_refresh.elapsed() >= app.refresh_interval {
            app.refresh_in_progress = true;
            if let Some(ro) = &db_ro {
                app.refresh(ro);
            } else if let Ok(db_guard) = db.try_lock() {
                app.refresh(&db_guard);
            }
            // If try_lock fails (backfill holds mutex), skip this tick.
            // The spinner stays visible and we retry on the next cycle.
        }
    }

    disable_raw_mode()?;
    std::io::stdout()
        .execute(DisableFocusChange)?
        .execute(DisableMouseCapture)?
        .execute(LeaveAlternateScreen)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_turn(input: i64) -> scopeon_core::Turn {
        scopeon_core::Turn {
            id: "t".into(),
            session_id: "s".into(),
            turn_index: 0,
            timestamp: 0,
            duration_ms: None,
            input_tokens: input,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cache_write_5m_tokens: 0,
            cache_write_1h_tokens: 0,
            output_tokens: 100,
            thinking_tokens: 0,
            mcp_call_count: 0,
            mcp_input_token_est: 0,
            text_output_tokens: 100,
            model: "claude-sonnet".into(),
            service_tier: "standard".into(),
            estimated_cost_usd: 0.01,
            is_compaction_event: false,
        }
    }

    #[test]
    fn test_predict_no_data() {
        assert_eq!(predict_turns_remaining_from_turns(&[], 100_000), None);
    }

    #[test]
    fn test_predict_too_few_turns() {
        let turns = vec![make_turn(1000), make_turn(2000)];
        assert_eq!(predict_turns_remaining_from_turns(&turns, 100_000), None);
    }

    #[test]
    fn test_predict_growing_context() {
        // 10 turns growing by 1000 tokens each — slope ≈ 1000
        let turns: Vec<_> = (1..=10).map(|i| make_turn(i * 1_000)).collect();
        let result = predict_turns_remaining_from_turns(&turns, 50_000);
        assert!(
            result.is_some(),
            "should return a prediction for growing context"
        );
        let predicted = result.unwrap();
        // With slope ~1000 and 50k remaining, expect roughly 50 turns
        assert!(predicted > 30 && predicted < 80, "predicted={}", predicted);
    }

    #[test]
    fn test_predict_flat_returns_none() {
        // All turns the same size — slope = 0
        let turns: Vec<_> = (0..5).map(|_| make_turn(5_000)).collect();
        assert_eq!(predict_turns_remaining_from_turns(&turns, 100_000), None);
    }

    #[test]
    fn test_predict_decreasing_returns_none() {
        // Decreasing turns (context shrinking — maybe after /compact)
        let turns: Vec<_> = (0..5).map(|i| make_turn(10_000 - i * 1_000)).collect();
        assert_eq!(predict_turns_remaining_from_turns(&turns, 100_000), None);
    }
}
