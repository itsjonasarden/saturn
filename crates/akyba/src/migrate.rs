//! Moving in from Saturn, as Akyba was called. On the first launch Saturn's settings folder
//! (settings, windows, sessions, routines, ports) becomes Akyba's, its status line in Claude Code
//! runs Akyba, and its Codex profile, which Akyba writes anew under its own name, is cleared away.
//! Nothing a person set up is lost; each step does nothing once done.

use std::path::PathBuf;

/// Everything above, before settings load. Only the app does it: a build run from the source tree
/// (`pnpm dev`, a spike) must never point someone's Claude Code at itself.
pub fn from_saturn() {
    let in_app = std::env::current_exe().is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"));
    if !in_app || crate::spike::is_automated() {
        return;
    }
    move_settings_folder();
    crate::status_line::point_here();
    if let Some(home) = akyba_core::agents::Agent::Codex.home() {
        let _ = std::fs::remove_file(home.join("saturn.config.toml"));
    }
}

/// Saturn's folder becomes Akyba's, when Akyba has none yet. Not for a test's own folder.
fn move_settings_folder() {
    if std::env::var_os("AKYBA_USER_DATA").is_some() {
        return;
    }
    let (Some(home), Some(akyba)) = (std::env::var_os("HOME"), crate::settings::data_dir()) else { return };
    let saturn = PathBuf::from(home).join("Library/Application Support/Saturn");
    if saturn.is_dir() && !akyba.exists() {
        let _ = std::fs::rename(&saturn, &akyba);
    }
}
