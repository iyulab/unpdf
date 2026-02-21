//! Self-update functionality using GitHub releases

use colored::Colorize;
use self_update::backends::github::{ReleaseList, Update};
use self_update::cargo_crate_version;
use semver::Version;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const REPO_OWNER: &str = "iyulab";
const REPO_NAME: &str = "unpdf";
const BIN_NAME: &str = "unpdf";
const CLI_CRATE_NAME: &str = "unpdf-cli";

/// Platform info for asset matching
struct PlatformInfo {
    /// Human-friendly OS name (windows, linux, macos)
    os_name: &'static str,
    /// Human-friendly arch name (x86_64, aarch64)
    arch_name: &'static str,
    /// Rust target triple (x86_64-pc-windows-msvc, etc.)
    target_triple: &'static str,
    /// Archive extension (zip for Windows, tar.gz for Unix)
    archive_ext: &'static str,
}

/// Get platform info for the current system
fn get_platform_info() -> PlatformInfo {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return PlatformInfo {
        os_name: "windows",
        arch_name: "x86_64",
        target_triple: "x86_64-pc-windows-msvc",
        archive_ext: "zip",
    };

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return PlatformInfo {
        os_name: "linux",
        arch_name: "x86_64",
        target_triple: "x86_64-unknown-linux-gnu",
        archive_ext: "tar.gz",
    };

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return PlatformInfo {
        os_name: "macos",
        arch_name: "x86_64",
        target_triple: "x86_64-apple-darwin",
        archive_ext: "tar.gz",
    };

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return PlatformInfo {
        os_name: "macos",
        arch_name: "aarch64",
        target_triple: "aarch64-apple-darwin",
        archive_ext: "tar.gz",
    };

    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
    )))]
    {
        // Fallback for unsupported platforms
        PlatformInfo {
            os_name: std::env::consts::OS,
            arch_name: std::env::consts::ARCH,
            target_triple: "unknown",
            archive_ext: "tar.gz",
        }
    }
}

/// Generate asset name patterns to search for (in priority order)
fn get_asset_patterns(platform: &PlatformInfo, version: &str) -> Vec<String> {
    let v = version.trim_start_matches('v');
    vec![
        // Human-friendly format (preferred): unpdf-windows-x86_64-v0.2.0.zip
        format!(
            "unpdf-{}-{}-v{}.{}",
            platform.os_name, platform.arch_name, v, platform.archive_ext
        ),
        // Without 'v' prefix: unpdf-windows-x86_64-0.2.0.zip
        format!(
            "unpdf-{}-{}-{}.{}",
            platform.os_name, platform.arch_name, v, platform.archive_ext
        ),
        // Target triple format: unpdf-x86_64-pc-windows-msvc-v0.2.0.zip
        format!(
            "unpdf-{}-v{}.{}",
            platform.target_triple, v, platform.archive_ext
        ),
        // Target triple without 'v': unpdf-x86_64-pc-windows-msvc-0.2.0.zip
        format!(
            "unpdf-{}-{}.{}",
            platform.target_triple, v, platform.archive_ext
        ),
    ]
}

/// Find matching asset name from a list of asset names using fallback patterns
fn find_matching_asset(asset_names: &[String], patterns: &[String]) -> Option<String> {
    for pattern in patterns {
        if asset_names.iter().any(|name| name == pattern) {
            return Some(pattern.clone());
        }
    }
    None
}

/// Get target strings to try for self_update matching (in priority order)
fn get_target_strings(platform: &PlatformInfo) -> Vec<String> {
    vec![
        // Human-friendly format: windows-x86_64
        format!("{}-{}", platform.os_name, platform.arch_name),
        // Target triple: x86_64-pc-windows-msvc
        platform.target_triple.to_string(),
    ]
}

/// Detect if installed via cargo install (binary in .cargo/bin)
fn is_cargo_install() -> bool {
    if let Ok(exe_path) = std::env::current_exe() {
        let path_str = exe_path.to_string_lossy();
        path_str.contains(".cargo") && path_str.contains("bin")
    } else {
        false
    }
}

/// Result of background update check
pub struct UpdateCheckResult {
    pub has_update: bool,
    pub latest_version: String,
    pub current_version: String,
}

/// Spawns a background thread to check for updates.
/// Returns a receiver that will contain the result when ready.
pub fn check_update_async() -> mpsc::Receiver<Option<UpdateCheckResult>> {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let result = check_latest_version();
        let _ = tx.send(result);
    });

    rx
}

/// Check for latest version without blocking (internal)
fn check_latest_version() -> Option<UpdateCheckResult> {
    let current_version = cargo_crate_version!();

    // Fetch releases from GitHub with timeout
    let releases = ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()
        .ok()?
        .fetch()
        .ok()?;

    if releases.is_empty() {
        return None;
    }

    let latest = &releases[0];
    let latest_version = latest.version.trim_start_matches('v');

    let current = Version::parse(current_version).ok()?;
    let latest_ver = Version::parse(latest_version).ok()?;

    Some(UpdateCheckResult {
        has_update: latest_ver > current,
        latest_version: latest_version.to_string(),
        current_version: current_version.to_string(),
    })
}

/// Try to receive update check result (non-blocking with short timeout)
pub fn try_get_update_result(
    rx: &mpsc::Receiver<Option<UpdateCheckResult>>,
) -> Option<UpdateCheckResult> {
    // Wait up to 500ms for the result
    rx.recv_timeout(Duration::from_millis(500)).ok().flatten()
}

/// Print update notification if new version available
pub fn print_update_notification(result: &UpdateCheckResult) {
    if result.has_update {
        println!();
        println!(
            "{} {} → {} available! Run '{}' to update.",
            "Update:".yellow().bold(),
            result.current_version,
            result.latest_version.green(),
            "unpdf update".cyan()
        );
    }
}

/// Run the update process
pub fn run_update(check_only: bool, force: bool) -> Result<(), Box<dyn std::error::Error>> {
    let current_version = cargo_crate_version!();
    println!("{} {}", "Current version:".cyan().bold(), current_version);

    println!("{}", "Checking for updates...".cyan());

    // Fetch releases from GitHub
    let releases = ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()?
        .fetch()?;

    if releases.is_empty() {
        println!("{}", "No releases found on GitHub.".yellow());
        return Ok(());
    }

    // Get latest release version
    let latest = &releases[0];
    let latest_version = latest.version.trim_start_matches('v');

    println!("{} {}", "Latest version:".cyan().bold(), latest_version);

    // Compare versions
    let current = Version::parse(current_version)?;
    let latest_ver = Version::parse(latest_version)?;

    if current >= latest_ver && !force {
        println!();
        println!("{} You are running the latest version!", "✓".green().bold());
        return Ok(());
    }

    if current < latest_ver {
        println!();
        println!(
            "{} New version available: {} → {}",
            "↑".yellow().bold(),
            current_version.yellow(),
            latest_version.green().bold()
        );
    }

    if check_only {
        println!();
        if is_cargo_install() {
            println!(
                "Run '{}' to update.",
                format!("cargo install {}", CLI_CRATE_NAME).cyan()
            );
        } else {
            println!("Run '{}' to update.", "unpdf update".cyan());
        }
        return Ok(());
    }

    // Check installation method
    if is_cargo_install() {
        println!();
        println!(
            "{} Installed via cargo. Please run:",
            "Note:".yellow().bold()
        );
        println!(
            "  {}",
            format!("cargo install {}", CLI_CRATE_NAME).cyan().bold()
        );
        println!();
        println!(
            "{}",
            "This ensures proper integration with your Rust toolchain.".dimmed()
        );
        return Ok(());
    }

    // Perform update (GitHub Releases only)
    println!();
    println!("{}", "Downloading update...".cyan());

    let platform = get_platform_info();
    let patterns = get_asset_patterns(&platform, latest_version);

    // Extract asset names from release
    let asset_names: Vec<String> = latest.assets.iter().map(|a| a.name.clone()).collect();

    // Find matching asset from release
    let asset_name = find_matching_asset(&asset_names, &patterns);

    if asset_name.is_none() {
        // Show what we searched for
        println!("{}", "No matching asset found.".red());
        println!("{}", "Searched for:".dimmed());
        for p in &patterns {
            println!("  - {}", p.dimmed());
        }
        println!();
        println!(
            "{} {}",
            "Available assets:".dimmed(),
            latest
                .assets
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Err("No compatible binary found for this platform".into());
    }

    let asset_name = asset_name.unwrap();
    println!("{} {}", "Found asset:".dimmed(), asset_name.dimmed());

    // Try multiple target strings for self_update matching
    let target_strings = get_target_strings(&platform);
    let mut last_error: Option<Box<dyn std::error::Error>> = None;

    for target in &target_strings {
        println!("{} target: {}", "Checking".dimmed(), target.dimmed());

        let result = Update::configure()
            .repo_owner(REPO_OWNER)
            .repo_name(REPO_NAME)
            .bin_name(BIN_NAME)
            .target(target)
            .current_version(current_version)
            .show_download_progress(true)
            .no_confirm(true)
            .build()
            .and_then(|updater| updater.update());

        match result {
            Ok(status) => {
                match status {
                    self_update::Status::UpToDate(v) => {
                        println!("{} Already up to date (v{})", "✓".green().bold(), v);
                    }
                    self_update::Status::Updated(v) => {
                        println!();
                        println!("{} Successfully updated to v{}!", "✓".green().bold(), v);
                        println!();
                        println!("Restart unpdf to use the new version.");
                    }
                }
                return Ok(());
            }
            Err(e) => {
                last_error = Some(Box::new(e));
                continue;
            }
        }
    }

    // All targets failed
    if let Some(e) = last_error {
        return Err(format!("Update failed: {}", e).into());
    }

    Ok(())
}
