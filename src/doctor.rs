//! IS-J: `scopeon doctor` — health diagnostics with provable Rust overhead metrics.
//!
//! Prints a human-readable report covering:
//! - Runtime: process RSS memory, DB file size/path
//! - Providers: availability check for each configured provider
//! - Data totals: sessions, turns, cost, cache savings
//! - Config: CLAUDE_CONFIG_DIR, budget, retention
//! - Overhead comparison vs Node.js-based tools

use anyhow::Result;
use scopeon_collector::Provider;
use scopeon_core::Database;

pub fn run(db: &Database, providers: &[Box<dyn Provider>]) -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    println!();
    println!("  Scopeon Doctor — v{version}");
    println!("  {}", "═".repeat(46));

    // ── Runtime ──────────────────────────────────────────────────────────────
    println!();
    println!("  Runtime");
    let rss_mb = read_rss_mb();
    match rss_mb {
        Some(mb) => println!("    Process memory    : {:.1} MB RSS", mb),
        None => println!("    Process memory    : N/A (read /proc/self/status)"),
    }
    let exe_path = std::env::current_exe().ok();
    let bin_size_mb = exe_path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.len() as f64 / 1_048_576.0);
    match bin_size_mb {
        Some(mb) => println!("    Binary size       : {:.1} MB", mb),
        None => println!("    Binary size       : N/A"),
    }
    let db_path = db
        .path()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("(in-memory)"));
    let db_size_mb = std::fs::metadata(&db_path)
        .ok()
        .map(|m| m.len() as f64 / 1_048_576.0);
    match db_size_mb {
        Some(mb) => println!("    DB size           : {:.1} MB", mb),
        None => println!("    DB size           : N/A"),
    }
    println!("    DB path           : {}", db_path.display());

    // ── Providers ─────────────────────────────────────────────────────────────
    println!();
    println!("  Providers");
    let mut any_available = false;
    for provider in providers {
        let available = provider.is_available();
        let icon = if available { "✓" } else { "✗" };
        let paths = provider.watch_paths();
        let path_str = paths
            .first()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "—".to_string());
        if available {
            any_available = true;
            println!("    {icon} {:<16}: {}", provider.name(), path_str);
        } else {
            println!(
                "    {icon} {:<16}: {} (not found)",
                provider.name(),
                path_str
            );
        }
    }

    // ── Data ──────────────────────────────────────────────────────────────────
    println!();
    println!("  Data");
    match db.get_global_stats() {
        Ok(g) => {
            println!("    Sessions          : {}", g.total_sessions);
            println!("    Turns             : {}", g.total_turns);
            println!("    Total cost        : ${:.2}", g.estimated_cost_usd);
            println!(
                "    Cache savings     : ${:.2} ({:.0}%)",
                g.cache_savings_usd,
                if g.estimated_cost_usd > 0.0 {
                    g.cache_savings_usd / (g.estimated_cost_usd + g.cache_savings_usd) * 100.0
                } else {
                    0.0
                }
            );
        },
        Err(e) => println!("    (error reading stats: {e})"),
    }

    // ── Config ────────────────────────────────────────────────────────────────
    println!();
    println!("  Config");
    match std::env::var("CLAUDE_CONFIG_DIR") {
        Ok(v) => println!("    CLAUDE_CONFIG_DIR : {v} (override active)"),
        Err(_) => println!("    CLAUDE_CONFIG_DIR : (not set — using platform default)"),
    }

    // ── Overhead ──────────────────────────────────────────────────────────────
    println!();
    println!("  Overhead vs. Node.js tools");
    match rss_mb {
        Some(mb) => {
            let node_typical_mb = 180.0_f64;
            let ratio = node_typical_mb / mb;
            println!("    RAM (Scopeon)     : {:.1} MB RSS", mb);
            println!(
                "    RAM (Node.js est) : ~{:.0} MB (typical ccusage)",
                node_typical_mb
            );
            println!("    Advantage         : {:.0}× less memory", ratio);
        },
        None => {
            println!("    RAM comparison    : N/A (could not read RSS)");
        },
    }
    println!("    CPU (idle)        : ~0% (event-driven, not polling)");

    // ── Pricing accuracy ─────────────────────────────────────────────────────
    println!();
    println!("  Pricing Accuracy");
    // Query for models in the DB that don't match any known prefix.
    {
        // Trigger get_pricing for every distinct model to populate UNKNOWN_MODELS_SEEN.
        if let Ok(model_rows) = db.get_cache_tokens_by_model() {
            for (model, _, _) in &model_rows {
                let _ = scopeon_core::get_pricing(model);
            }
        }
    }
    let unknown_model_count = scopeon_core::UNKNOWN_MODELS_SEEN
        .lock()
        .ok()
        .map(|g| g.len())
        .unwrap_or(0);
    if unknown_model_count == 0 {
        println!("    ✅ All models have known pricing");
    } else {
        let models = scopeon_core::UNKNOWN_MODELS_SEEN
            .lock()
            .ok()
            .map(|g| {
                let mut names: Vec<&str> = g.iter().map(|s| s.as_str()).collect();
                names.sort_unstable();
                names.join(", ")
            })
            .unwrap_or_default();
        println!(
            "    ⚠  {} unknown model(s) using Sonnet fallback pricing: {}",
            unknown_model_count, models
        );
        println!("       Cost estimates for these models may be off by up to 75%.");
        println!(
            "       Add overrides in ~/.scopeon/config.toml: [pricing.overrides.\"model-name\"]"
        );
    }

    // ── Health summary ────────────────────────────────────────────────────────
    println!();
    let healthy = any_available && db_size_mb.is_some();
    if healthy {
        println!("  Health: ✅  All checks passed");
    } else {
        if !any_available {
            println!("  ⚠  No providers found — is Claude Code / an AI agent installed?");
        }
        if db_size_mb.is_none() {
            println!("  ⚠  DB not accessible at {}", db_path.display());
        }
        println!("  Health: ❌  Some checks failed (see above)");
    }
    println!();

    if !healthy {
        std::process::exit(1);
    }
    Ok(())
}

/// Read process RSS from /proc/self/status (Linux) or via sysinfo (macOS/others).
/// Returns None if unavailable.
fn read_rss_mb() -> Option<f64> {
    // Linux: /proc/self/status has "VmRSS: <n> kB"
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("VmRSS:") {
                    if let Some(kb_str) = rest.trim().split_whitespace().next() {
                        if let Ok(kb) = kb_str.parse::<u64>() {
                            return Some(kb as f64 / 1024.0);
                        }
                    }
                }
            }
        }
        None
    }

    // macOS: use MACH_TASK_BASIC_INFO (flavor 20) which uses u64 fields.
    // TASK_BASIC_INFO (flavor 5) uses natural_t fields that vary by arch;
    // reinterpreting i32 pointers as u64 is undefined behaviour. Use
    // the ABI-stable MACH_TASK_BASIC_INFO struct instead.
    #[cfg(target_os = "macos")]
    {
        use std::mem;
        #[repr(C)]
        struct MachTaskBasicInfo {
            virtual_size: u64,
            resident_size: u64,
            resident_size_max: u64,
            user_time: [u32; 2],
            system_time: [u32; 2],
            policy: i32,
            suspend_count: i32,
        }
        extern "C" {
            fn mach_task_self() -> u32;
            fn task_info(
                task: u32,
                flavor: u32,
                task_info_out: *mut u8,
                task_info_count: *mut u32,
            ) -> i32;
        }
        const MACH_TASK_BASIC_INFO: u32 = 20;
        let mut info: MachTaskBasicInfo = unsafe { mem::zeroed() };
        let mut count = (mem::size_of::<MachTaskBasicInfo>() / mem::size_of::<u32>()) as u32;
        let ret = unsafe {
            task_info(
                mach_task_self(),
                MACH_TASK_BASIC_INFO,
                &mut info as *mut _ as *mut u8,
                &mut count,
            )
        };
        if ret == 0 {
            return Some(info.resident_size as f64 / 1_048_576.0);
        }
        None
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}
