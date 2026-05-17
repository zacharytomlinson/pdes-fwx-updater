//! PDES Firmware Updater application entrypoint and UI.
//! Author: Zachary Tomlinson <ztomlinson7@gmail.com>
//! Version: v1.0.0
//! Date: 2025-02-12
//! Cross-platform egui/eframe desktop UI for flashing PDES firmware, handling
//! probe and USB vectors plus installer bootstrap per platform.
#![feature(trim_prefix_suffix)]
#![windows_subsystem = "windows"] // hide console window on Windows in release

mod app_dirs;
mod bootstrap;
mod device_token;
mod downloader;
mod flasher;
mod manifest;
mod package;
mod seed;
mod settings;
mod updater;

use crate::downloader::{
    download_app_pkg_to_staging_verified, download_to_app_firmware_verified,
    find_cached_firmware_by_sha256, find_latest_cached_firmware,
};
use crate::flasher::{ChipModel, FlashVector};
use crate::manifest::UpdateIdentity;
use crate::package::{extract_pkg_and_find_installer, launch_installer};
use crate::settings::AppSettings;
use crate::updater::UpdateCheckState;
use eframe::epaint::Color32;
use eframe::{App, CreationContext, NativeOptions, egui};
use egui::{Align2, Button, CentralPanel, RichText, ScrollArea, Stroke, TopBottomPanel, Vec2};
use egui_console::{ConsoleBuilder, ConsoleWindow};
use egui_file_dialog::FileDialog;
use flasher::Flasher;
use probe_rs::probe::list::Lister;
use serialport::{SerialPortInfo, SerialPortType};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::runtime::Runtime;
use tokio::time::sleep;

#[derive(Default, Debug, Copy, Clone)]
pub enum ViewState {
    #[default]
    Splash,
    Main,
}

#[derive(Debug, Clone)]
enum FlashOutcome {
    Success,
    Failure(String),
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum AppStatus {
    Idle,
    InProgress,
    Success,
    Failure(String),
}

#[derive(Debug, Default, Clone)]
struct PreflightReport {
    checked_at: Option<Instant>,
    bossac_path: String,
    bossac_bundled: bool,
    bossac_from_path: bool,
    serial_ports: Vec<String>,
    serial_error: Option<String>,
    probes: Vec<String>,
    probe_error: Option<String>,
}

#[derive(Debug, Clone)]
enum UiEvent {
    SetFirmwarePath(PathBuf),
    BootstrapComplete(crate::bootstrap::BootstrapExchangeResult),
    BootstrapFailed(String),
    UpdateResultAvailable(crate::updater::UpdateResult),
    FirmwareEnsureFinished,
    ExitRequested,
}

const UI_CENTER_COLUMN_MAX_WIDTH: f32 = 900.0;
const UI_CONTROLS_DEFAULT_HEIGHT: f32 = 420.0;
const UI_CONTROLS_MIN_HEIGHT: f32 = 220.0;
const UI_LOGO_MAX_HEIGHT: f32 = 80.0;
const UI_FLASH_PANEL_HEIGHT: f32 = 96.0;
const UI_FLASH_BUTTON_SIZE: Vec2 = Vec2::new(160.0, 42.0);
const SPLASH_MIN_OUTCOME_DURATION: Duration = Duration::from_millis(1500);
const LINUX_DESKTOP_APP_ID: &str = "pdes";

fn companion_bg() -> Color32 {
    Color32::from_rgb(11, 18, 34)
}

fn companion_panel() -> Color32 {
    Color32::from_rgb(17, 24, 39)
}

fn companion_panel_soft() -> Color32 {
    Color32::from_rgb(15, 23, 42)
}

fn companion_input() -> Color32 {
    Color32::from_rgb(11, 20, 37)
}

fn companion_border() -> Color32 {
    Color32::from_rgb(31, 41, 55)
}

fn companion_text() -> Color32 {
    Color32::from_rgb(226, 232, 240)
}

fn companion_muted() -> Color32 {
    Color32::from_rgb(203, 213, 225)
}

fn companion_accent() -> Color32 {
    Color32::from_rgb(14, 165, 233)
}

fn companion_accent_strong() -> Color32 {
    Color32::from_rgb(6, 182, 212)
}

fn companion_success() -> Color32 {
    Color32::from_rgb(34, 197, 94)
}

fn companion_warn() -> Color32 {
    Color32::from_rgb(239, 68, 68)
}

fn companion_panel_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(companion_panel())
        .corner_radius(16)
        .stroke(Stroke::new(1.0_f32, companion_border()))
        .inner_margin(egui::Margin::same(18))
}

fn companion_soft_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(companion_panel_soft())
        .corner_radius(12)
        .stroke(Stroke::new(1.0_f32, companion_border()))
        .inner_margin(egui::Margin::same(12))
}

fn apply_companion_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = companion_bg();
    visuals.window_fill = companion_panel();
    visuals.extreme_bg_color = companion_input();
    visuals.faint_bg_color = companion_panel_soft();
    visuals.hyperlink_color = companion_accent();
    visuals.override_text_color = Some(companion_text());
    visuals.selection.bg_fill = companion_accent();
    visuals.selection.stroke = Stroke::new(1.0_f32, companion_text());
    visuals.widgets.noninteractive.bg_fill = companion_panel();
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, companion_text());
    visuals.widgets.inactive.bg_fill = companion_input();
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, companion_border());
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, companion_text());
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(12, 35, 58);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, companion_accent());
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, companion_text());
    visuals.widgets.active.bg_fill = Color32::from_rgb(8, 47, 73);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, companion_accent_strong());
    visuals.widgets.open.bg_fill = companion_input();
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, companion_accent());
    visuals.window_stroke = Stroke::new(1.0_f32, companion_border());
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 10.0);
    style.spacing.button_padding = Vec2::new(14.0, 9.0);
    style.spacing.interact_size = Vec2::new(42.0, 34.0);
    ctx.set_style(style);
}

fn status_badge(ui: &mut egui::Ui, text: impl Into<String>, color: Color32) {
    egui::Frame::NONE
        .fill(Color32::from_rgba_unmultiplied(
            color.r(),
            color.g(),
            color.b(),
            32,
        ))
        .corner_radius(10)
        .stroke(Stroke::new(
            1.0_f32,
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 110),
        ))
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(RichText::new(text.into()).size(12.0).strong().color(color));
        });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SplashHoldKey {
    UpToDate,
    Error,
}

fn next_bootstrap_backoff(failures: u32) -> Duration {
    let exp = failures.min(5);
    let mut secs = 2u64.saturating_mul(1u64 << exp);
    secs = secs.min(60);
    let jitter_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| (d.subsec_millis() % 500) as u64)
        .unwrap_or(0);
    Duration::from_secs(secs) + Duration::from_millis(jitter_ms)
}

fn format_hw_rev(hw_rev: Option<u32>) -> String {
    hw_rev
        .map(|value| format!("0x{value:08X}"))
        .unwrap_or_else(|| "unset".to_string())
}

fn is_updater_managed_firmware_path(path: &Path) -> bool {
    crate::app_dirs::firmware_dir()
        .ok()
        .is_some_and(|dir| path.starts_with(dir))
}

fn resolve_firmware_for_detected_board(
    update_identity: &UpdateIdentity,
    token: Option<&str>,
    hw_rev: Option<u32>,
    current_path: &Path,
    ot_tx: &Sender<String>,
    ui_tx: &Sender<UiEvent>,
) -> anyhow::Result<Option<PathBuf>> {
    let Some(token) = token.filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    if !is_updater_managed_firmware_path(current_path) {
        return Ok(None);
    }

    let firmware_url = update_identity.firmware_latest_url(hw_rev);
    let Some(artifact) = updater::fetch_firmware_update(&firmware_url, token)? else {
        return Ok(None);
    };

    let version = artifact.latest_version.trim().to_string();
    let Some(download_url) = artifact.download_url else {
        let _ = ot_tx.send(format!(
            "Firmware metadata for board {} is missing a download URL; keeping current file.",
            format_hw_rev(hw_rev)
        ));
        return Ok(None);
    };
    let Some(expected_sha) = artifact.sha256 else {
        let _ = ot_tx.send(format!(
            "Firmware metadata for board {} is missing sha256; keeping current file.",
            format_hw_rev(hw_rev)
        ));
        return Ok(None);
    };

    if let Some(path) = find_cached_firmware_by_sha256(&expected_sha)? {
        let _ = ot_tx.send(format!(
            "Using cached firmware {} for board {}.",
            if version.is_empty() {
                "artifact".to_string()
            } else {
                version
            },
            format_hw_rev(hw_rev)
        ));
        let _ = ui_tx.send(UiEvent::SetFirmwarePath(path.clone()));
        return Ok(Some(path));
    }

    let downloaded = download_to_app_firmware_verified(&download_url, token, &expected_sha)?;
    let _ = ot_tx.send(format!(
        "Downloaded firmware {} for board {} to {}.",
        if version.is_empty() {
            "artifact".to_string()
        } else {
            version
        },
        format_hw_rev(hw_rev),
        downloaded.display()
    ));
    let _ = ui_tx.send(UiEvent::SetFirmwarePath(downloaded.clone()));
    Ok(Some(downloaded))
}

//the main application struct, holding its current screen
struct FWUpdater {
    state: Arc<Mutex<ViewState>>,
    load_progress: Arc<Mutex<f32>>,
    flasher: Arc<Mutex<Flasher>>,
    elf_path: Arc<Mutex<Option<PathBuf>>>,
    fd: Arc<Mutex<FileDialog>>,
    fd_text: String,
    console: ConsoleWindow,
    show_output: bool,

    lp_tx: Sender<f32>,
    lp_rx: Receiver<f32>,
    stat_tx: Sender<ViewState>,
    stat_rx: Receiver<ViewState>,
    ot_tx: Sender<String>,
    ot_rx: Receiver<String>,
    result_tx: Sender<FlashOutcome>,
    result_rx: Receiver<FlashOutcome>,
    flash_result: Option<FlashOutcome>,
    status_tx: Sender<AppStatus>,
    status_rx: Receiver<AppStatus>,
    status: AppStatus,
    pending_result: Option<FlashOutcome>,
    last_output: Instant,
    preflight: PreflightReport,
    preflight_open: bool,

    settings: AppSettings,
    token_prompt_open: bool,
    token_prompt_dismissed: bool,
    token_input: String,
    update_identity: UpdateIdentity,
    app_version: String,
    splash_status: String,
    splash_app_update_in_flight: bool,
    splash_hold_until: Option<Instant>,
    splash_hold_key: Option<SplashHoldKey>,
    splash_firmware_in_flight: bool,
    splash_firmware_done: bool,
    exit_requested: bool,

    update_state: UpdateCheckState,
    last_update_result: Option<crate::updater::UpdateResult>,
    update_check_started: bool,
    bootstrap_in_flight: bool,
    bootstrap_retry_at: Option<Instant>,
    bootstrap_failures: u32,
    device_token: Option<String>,
    update_tx: Sender<UpdateCheckState>,
    update_rx: Receiver<UpdateCheckState>,

    ui_tx: Sender<UiEvent>,
    ui_rx: Receiver<UiEvent>,
}

impl Default for FWUpdater {
    fn default() -> Self {
        let (lp_tx, lp_rx) = channel();
        let (stat_tx, stat_rx) = channel();
        let (ot_tx, ot_rx) = channel();
        let (result_tx, result_rx) = channel();
        let (status_tx, status_rx) = channel();
        let (update_tx, update_rx) = channel();
        let (ui_tx, ui_rx) = channel();
        let settings = AppSettings::load_or_default();
        let token_input = settings.update_token.clone().unwrap_or_default();
        let dirs_outcome = crate::app_dirs::ensure_dirs();
        let seed_outcome = crate::seed::ensure_seed_config();
        let update_identity = UpdateIdentity::load_or_default();
        let app_version = crate::manifest::load_manifest_app_version()
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
        let mut app = Self {
            state: Arc::new(Mutex::new(ViewState::Splash)),
            load_progress: Arc::new(Mutex::new(0.0)),
            flasher: Default::default(),
            elf_path: Arc::new(Mutex::new(None)),
            fd: Default::default(),
            fd_text: "Select Binary to Flash".to_string(),
            show_output: false,
            flash_result: None,
            pending_result: None,
            last_output: Instant::now(),
            status: AppStatus::Idle,
            console: ConsoleBuilder::new()
                .prompt("PDES Flasher starting...")
                .history_size(1000)
                .scrollback_size(1000)
                .build(),

            lp_rx,
            lp_tx,
            stat_tx,
            stat_rx,
            ot_tx,
            ot_rx,
            result_tx,
            result_rx,
            status_tx,
            status_rx,
            preflight: PreflightReport::default(),
            preflight_open: true,
            settings,
            token_prompt_open: false,
            token_prompt_dismissed: false,
            token_input,
            update_identity,
            app_version,
            splash_status: "Starting...".to_string(),
            splash_app_update_in_flight: false,
            splash_hold_until: None,
            splash_hold_key: None,
            splash_firmware_in_flight: false,
            splash_firmware_done: false,
            exit_requested: false,
            update_state: UpdateCheckState::NotConfigured,
            last_update_result: None,
            update_check_started: false,
            bootstrap_in_flight: false,
            bootstrap_retry_at: None,
            bootstrap_failures: 0,
            device_token: None,
            update_tx,
            update_rx,
            ui_tx,
            ui_rx,
        };

        if let Err(e) = dirs_outcome {
            let msg = format!("Unable to initialize persistent storage: {e:#}");
            app.console.write(&msg);
        }

        match seed_outcome {
            Ok(Some(path)) => {
                let msg = format!("Seeded updater config at {}", path.display());
                app.console.write(&msg);
            }
            Ok(None) => {}
            Err(e) => {
                let msg = format!("Unable to seed updater config: {e:#}");
                app.console.write(&msg);
            }
        }

        if let Ok(Some(tok)) = crate::device_token::load() {
            app.device_token = Some(tok);
            app.console.write("Loaded device token.");
        }

        if app
            .update_identity
            .bootstrap_pkg_url
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        {
            app.console.write("Bootstrap package URL configured.");
        }

        app
    }
}

impl FWUpdater {
    /// Construct the UI with default state.
    fn new(_cc: &CreationContext<'_>) -> Self {
        Default::default()
    }

    fn ensure_preflight(&mut self) {
        if self.preflight.checked_at.is_none() {
            self.refresh_preflight();
        }
    }

    fn ensure_updater(&mut self, allow_prompt: bool) {
        let token = self.effective_update_token();
        if token.is_none() {
            if self.maybe_start_bootstrap_exchange() {
                return;
            }
            self.update_state = UpdateCheckState::NotConfigured;
            if allow_prompt && !self.token_prompt_dismissed {
                self.token_prompt_open = true;
            }
            return;
        }

        if matches!(self.update_state, UpdateCheckState::NotConfigured) {
            self.update_state = UpdateCheckState::Checking;
        }

        if !self.update_check_started {
            self.update_check_started = true;
            self.start_update_check(token.expect("token checked above"));
        }
    }

    fn start_update_check(&mut self, token: String) {
        let package_url = self.update_identity.package_latest_url();
        let firmware_url = self.update_identity.firmware_latest_url(None);

        let tx = self.update_tx.clone();
        let ot_tx = self.ot_tx.clone();
        let ui_tx = self.ui_tx.clone();
        let current_version = env!("CARGO_PKG_VERSION").to_string();
        std::thread::spawn(move || {
            let _ = tx.send(UpdateCheckState::Checking);
            match updater::check_for_updates(&package_url, &firmware_url, &token) {
                Ok(result) => {
                    let _ = ui_tx.send(UiEvent::UpdateResultAvailable(result.clone()));
                    let app_update_available = result.app.as_ref().is_some_and(|a| {
                        updater::is_newer_version(&current_version, &a.latest_version)
                    });
                    let firmware_update_available = result.firmware.is_some();
                    if app_update_available || firmware_update_available {
                        let _ = tx.send(UpdateCheckState::UpdateAvailable(result));
                    } else {
                        let _ = tx.send(UpdateCheckState::UpToDate);
                    }
                }
                Err(e) => {
                    let _ = ot_tx.send(format!("Update check failed: {e:#}"));
                    let _ = tx.send(UpdateCheckState::Error(e.to_string()));
                }
            }
        });
    }

    fn start_app_update_install(&mut self, url: String, token: String, expected_sha: String) {
        if self.splash_app_update_in_flight {
            return;
        }
        self.splash_app_update_in_flight = true;

        let ot_tx = self.ot_tx.clone();
        let lp_tx = self.lp_tx.clone();
        let stat_tx = self.stat_tx.clone();
        let ui_tx = self.ui_tx.clone();

        std::thread::spawn(move || {
            let _ = lp_tx.send(0.35);
            let _ = ot_tx.send("Downloading update package...".to_string());
            let pkg_path = match download_app_pkg_to_staging_verified(&url, &token, &expected_sha) {
                Ok(path) => path,
                Err(e) => {
                    let _ = ot_tx.send(format!("Package download failed: {e:#}"));
                    let _ = stat_tx.send(ViewState::Main);
                    return;
                }
            };

            let _ = lp_tx.send(0.65);
            let _ = ot_tx.send(format!(
                "Package downloaded and verified: {}",
                pkg_path.display()
            ));

            let plan = match extract_pkg_and_find_installer(&pkg_path) {
                Ok(plan) => plan,
                Err(e) => {
                    let _ = ot_tx.send(format!("Unable to extract package: {e:#}"));
                    let _ = stat_tx.send(ViewState::Main);
                    return;
                }
            };

            let _ = lp_tx.send(0.85);
            let _ = ot_tx.send(format!(
                "Launching installer: {}",
                plan.installer_path.display()
            ));
            if let Err(e) = launch_installer(&plan.installer_path) {
                let _ = ot_tx.send(format!("Unable to launch installer: {e:#}"));
                let _ = stat_tx.send(ViewState::Main);
                return;
            }

            let _ = lp_tx.send(1.0);
            let _ = ot_tx.send(
                "Update installer launched. Closing PDES so the update can complete.".to_string(),
            );
            let _ = ui_tx.send(UiEvent::ExitRequested);
        });
    }

    fn splash_delay_elapsed(&mut self, ctx: &egui::Context, key: SplashHoldKey) -> bool {
        if self.splash_hold_key != Some(key) {
            self.splash_hold_key = Some(key);
            self.splash_hold_until = None;
        }
        let hold_until = self
            .splash_hold_until
            .get_or_insert_with(|| Instant::now() + SPLASH_MIN_OUTCOME_DURATION);
        if Instant::now() < *hold_until {
            ctx.request_repaint();
            return false;
        }
        true
    }

    fn ensure_startup_firmware_ready(&mut self, ctx: &egui::Context) -> bool {
        if self.splash_firmware_done {
            return true;
        }
        if self.splash_firmware_in_flight {
            ctx.request_repaint();
            return false;
        }

        let latest = self
            .last_update_result
            .as_ref()
            .and_then(|r| r.firmware.as_ref());

        // If we can't determine the latest firmware from the server, fall back to the newest cached
        // firmware (if present) and proceed.
        let (url, expected_sha) = match latest {
            Some(fw) => {
                let Some(url) = fw.download_url.as_deref() else {
                    if let Ok(Some(path)) = find_latest_cached_firmware() {
                        let _ = self.ui_tx.send(UiEvent::SetFirmwarePath(path));
                    }
                    self.splash_firmware_done = true;
                    return true;
                };
                let Some(sha) = fw.sha256.as_deref() else {
                    if let Ok(Some(path)) = find_latest_cached_firmware() {
                        let _ = self.ui_tx.send(UiEvent::SetFirmwarePath(path));
                    }
                    self.splash_firmware_done = true;
                    return true;
                };
                (url.to_string(), sha.to_string())
            }
            None => {
                if let Ok(Some(path)) = find_latest_cached_firmware() {
                    let _ = self.ui_tx.send(UiEvent::SetFirmwarePath(path));
                }
                self.splash_firmware_done = true;
                return true;
            }
        };

        // If we already have the expected firmware cached, select it and proceed.
        if let Ok(Some(path)) = find_cached_firmware_by_sha256(&expected_sha) {
            let _ = self.ui_tx.send(UiEvent::SetFirmwarePath(path));
            self.splash_firmware_done = true;
            return true;
        }

        let Some(token) = self.effective_update_token() else {
            if let Ok(Some(path)) = find_latest_cached_firmware() {
                let _ = self.ui_tx.send(UiEvent::SetFirmwarePath(path));
            }
            self.splash_firmware_done = true;
            return true;
        };

        self.splash_firmware_in_flight = true;
        let ot_tx = self.ot_tx.clone();
        let lp_tx = self.lp_tx.clone();
        let ui_tx = self.ui_tx.clone();
        std::thread::spawn(move || {
            let _ = lp_tx.send(0.45);
            let _ = ot_tx.send("Checking cached firmware...".to_string());
            let cached = find_cached_firmware_by_sha256(&expected_sha).ok().flatten();
            if let Some(path) = cached {
                let _ = ot_tx.send("Using cached firmware.".to_string());
                let _ = ui_tx.send(UiEvent::SetFirmwarePath(path));
                let _ = ui_tx.send(UiEvent::FirmwareEnsureFinished);
                return;
            }

            let _ = lp_tx.send(0.7);
            let _ = ot_tx.send("Downloading latest firmware...".to_string());
            match download_to_app_firmware_verified(&url, &token, &expected_sha) {
                Ok(path) => {
                    let _ = ot_tx.send(format!("Downloaded firmware to {}", path.display()));
                    let _ = ui_tx.send(UiEvent::SetFirmwarePath(path));
                }
                Err(e) => {
                    let _ = ot_tx.send(format!("Firmware download failed: {e:#}"));
                }
            }
            let _ = lp_tx.send(1.0);
            let _ = ui_tx.send(UiEvent::FirmwareEnsureFinished);
        });
        ctx.request_repaint();
        false
    }

    fn update_splash(&mut self, ctx: &egui::Context) {
        self.ensure_updater(false);

        let state = self.update_state.clone();
        match state {
            UpdateCheckState::NotConfigured => {
                self.splash_hold_until = None;
                self.splash_hold_key = None;
                self.splash_status = "Updater not configured; continuing.".to_string();
                let _ = self.lp_tx.send(1.0);
                self.splash_firmware_done = true;
                let _ = self.stat_tx.send(ViewState::Main);
            }
            UpdateCheckState::Checking => {
                self.splash_hold_until = None;
                self.splash_hold_key = None;
                self.splash_status = "Checking for updates...".to_string();
                let _ = self.lp_tx.send(0.2);
                ctx.request_repaint();
            }
            UpdateCheckState::UpToDate => {
                self.splash_status = "Up to date.".to_string();
                let _ = self.lp_tx.send(1.0);
                if !self.splash_delay_elapsed(ctx, SplashHoldKey::UpToDate) {
                    return;
                }
                if !self.ensure_startup_firmware_ready(ctx) {
                    return;
                }
                let _ = self.stat_tx.send(ViewState::Main);
            }
            UpdateCheckState::Error(e) => {
                self.splash_status = format!("Update check failed; continuing. ({})", e);
                let _ = self.lp_tx.send(1.0);
                if !self.splash_delay_elapsed(ctx, SplashHoldKey::Error) {
                    return;
                }
                // Can't trust remote metadata; still try to select the newest cached firmware.
                if !self.splash_firmware_done {
                    if let Ok(Some(path)) = find_latest_cached_firmware() {
                        let _ = self.ui_tx.send(UiEvent::SetFirmwarePath(path));
                    }
                    self.splash_firmware_done = true;
                }
                let _ = self.stat_tx.send(ViewState::Main);
            }
            UpdateCheckState::UpdateAvailable(result) => {
                self.last_update_result = Some(result.clone());
                self.splash_hold_until = None;
                self.splash_hold_key = None;
                let current_version = env!("CARGO_PKG_VERSION");
                let app_update = result
                    .app
                    .as_ref()
                    .is_some_and(|a| updater::is_newer_version(current_version, &a.latest_version));

                if !app_update {
                    self.splash_status = "No app update; continuing.".to_string();
                    let _ = self.lp_tx.send(1.0);
                    if !self.ensure_startup_firmware_ready(ctx) {
                        return;
                    }
                    let _ = self.stat_tx.send(ViewState::Main);
                    return;
                }

                let Some(app) = result.app else {
                    self.splash_status =
                        "Update metadata missing app payload; continuing.".to_string();
                    let _ = self.lp_tx.send(1.0);
                    let _ = self.stat_tx.send(ViewState::Main);
                    return;
                };

                let Some(url) = app.download_url else {
                    self.splash_status =
                        "Update metadata missing download URL; continuing.".to_string();
                    let _ = self.lp_tx.send(1.0);
                    let _ = self.stat_tx.send(ViewState::Main);
                    return;
                };
                let Some(expected_sha) = app.sha256 else {
                    self.splash_status = "Update metadata missing sha256; continuing.".to_string();
                    let _ = self.lp_tx.send(1.0);
                    let _ = self.stat_tx.send(ViewState::Main);
                    return;
                };
                let Some(token) = self.effective_update_token() else {
                    self.splash_status = "Updater token missing; continuing.".to_string();
                    let _ = self.lp_tx.send(1.0);
                    let _ = self.stat_tx.send(ViewState::Main);
                    return;
                };

                self.splash_status = format!(
                    "Update available: {} → {}",
                    current_version, app.latest_version
                );
                self.start_app_update_install(url, token, expected_sha);
                ctx.request_repaint();
            }
        }
    }

    fn effective_update_token(&mut self) -> Option<String> {
        if let Some(tok) = self
            .settings
            .update_token
            .clone()
            .filter(|s| !s.trim().is_empty())
        {
            return Some(tok);
        }
        if let Some(tok) = self.device_token.clone().filter(|s| !s.trim().is_empty()) {
            return Some(tok);
        }
        if let Ok(Some(tok)) = crate::device_token::load() {
            self.device_token = Some(tok.clone());
            return Some(tok);
        }
        None
    }

    fn maybe_start_bootstrap_exchange(&mut self) -> bool {
        if self.bootstrap_in_flight {
            return true;
        }
        if let Some(at) = self.bootstrap_retry_at
            && Instant::now() < at
        {
            return true;
        }
        let Some(pkg_url) = self
            .update_identity
            .bootstrap_pkg_url
            .clone()
            .filter(|s| !s.trim().is_empty())
        else {
            return false;
        };

        self.bootstrap_in_flight = true;
        self.update_state = UpdateCheckState::Checking;
        self.console
            .write("Exchanging bootstrap token for device token...");

        let ot_tx = self.ot_tx.clone();
        let ui_tx = self.ui_tx.clone();
        let update_tx = self.update_tx.clone();
        std::thread::spawn(move || {
            let _ = update_tx.send(UpdateCheckState::Checking);
            match crate::bootstrap::exchange_pkg_url(&pkg_url) {
                Ok(result) => match crate::device_token::store(&result.device_token) {
                    Ok(outcome) => {
                        let _ = ot_tx
                            .send(format!("Stored device token at {}", outcome.path.display()));
                        if cfg!(windows) && !outcome.perms_tightened {
                            let _ = ot_tx.send(
                                "WARNING: Unable to tighten device token ACLs (relying on LocalAppData defaults)."
                                    .to_string(),
                            );
                        }
                        match crate::seed::persist_bootstrap_exchange(&pkg_url, &result) {
                            Ok(cfg_path) => {
                                let _ = ot_tx.send(format!(
                                    "Updated updater config at {}",
                                    cfg_path.display()
                                ));
                            }
                            Err(e) => {
                                let _ =
                                    ot_tx.send(format!("Unable to persist updater config: {e}"));
                            }
                        }
                        let _ = ui_tx.send(UiEvent::BootstrapComplete(result));
                    }
                    Err(e) => {
                        let msg = "Unable to store device token.".to_string();
                        let _ = ot_tx.send(format!("{msg} ({e})"));
                        let _ = update_tx.send(UpdateCheckState::Error(msg.clone()));
                        let _ = ui_tx.send(UiEvent::BootstrapFailed(msg));
                    }
                },
                Err(e) => {
                    let msg = e.to_string();
                    let _ = ot_tx.send("Bootstrap exchange failed.".to_string());
                    let _ = update_tx.send(UpdateCheckState::Error(msg.clone()));
                    let _ = ui_tx.send(UiEvent::BootstrapFailed(msg));
                }
            }
        });
        true
    }

    fn refresh_preflight(&mut self) {
        self.console.write("Running pre-flight checks...");

        let bossac_path = Flasher::bossac_path();
        let bossac_from_path = bossac_path == std::path::Path::new("bossac");
        let bossac_bundled = !bossac_from_path && bossac_path.exists();

        let mut serial_ports = Vec::new();
        let mut serial_error = None;
        match serialport::available_ports() {
            Ok(ports) => {
                for port in ports {
                    serial_ports.push(format_serial_port(&port));
                }
            }
            Err(e) => {
                serial_error = Some(e.to_string());
            }
        }

        let mut probes = Vec::new();
        let lister = Lister::new();
        let list = lister.list_all();
        for probe in list {
            probes.push(probe.identifier);
        }

        self.preflight = PreflightReport {
            checked_at: Some(Instant::now()),
            bossac_path: bossac_path.display().to_string(),
            bossac_bundled,
            bossac_from_path,
            serial_ports,
            serial_error,
            probes,
            probe_error: None,
        };
    }

    /// Start flashing operation, dispatching to probe or USB depending on flash vector.
    fn start_flash_op(&mut self, mut flash: Flasher, path: String, ctx: egui::Context) {
        let prog = *self.load_progress.clone().lock().unwrap();
        let ot_tx = self.ot_tx.clone();
        let lp_tx = self.lp_tx.clone();
        let result_tx = self.result_tx.clone();
        let status_tx = self.status_tx.clone();
        let ui_tx = self.ui_tx.clone();
        let update_identity = self.update_identity.clone();
        let update_token = self.effective_update_token();
        let _ = status_tx.send(AppStatus::InProgress);
        tokio::spawn(async move {
            if flash.flash_vector == FlashVector::Probe {
                let _ = ot_tx.send("Connecting to Probe...".to_string());
                ctx.request_repaint();
                match flash.connect_probe() {
                    Ok(probe) => {
                        let _ = lp_tx.send(prog + 40.0);
                        let _ = ot_tx.send("Attaching Probe to Target...".to_string());
                        ctx.request_repaint();
                        match flash.attach_probe(probe) {
                            Ok(session) => {
                                let _ = lp_tx.send(prog + 5.0);
                                let _ = ot_tx.send("Flashing Binary to Target".to_string());
                                ctx.request_repaint();
                                match flash.flash_via_probe(session, path, |evt| {
                                    let _ = ot_tx.send(format!("{:#?}", evt));
                                    ctx.request_repaint();
                                }) {
                                    Ok(_) => {
                                        let _ = lp_tx.send(prog + 5.0);
                                        let _ = ot_tx.send("Flashing Successful!".to_string());
                                        ctx.request_repaint();
                                        let _ = result_tx.send(FlashOutcome::Success);
                                        let _ = status_tx.send(AppStatus::Success);
                                    }
                                    Err(e) => {
                                        let _ = ot_tx.send(e.to_string());
                                        let _ = lp_tx.send(0.0);
                                        ctx.request_repaint();
                                        let _ = sleep(Duration::from_secs(10)).await;
                                        let _ = ot_tx.send("Flashing failed... Did you try unplugging it and plugging it back in?".to_string());
                                        let _ =
                                            result_tx.send(FlashOutcome::Failure(e.to_string()));
                                        let _ = status_tx.send(AppStatus::Failure(e.to_string()));
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = ot_tx.send(e.to_string());
                                let _ = lp_tx.send(0.0);
                                ctx.request_repaint();
                                let _ = sleep(Duration::from_secs(10)).await;
                                let _ = ot_tx.send("Flashing failed... Did you try unplugging it and plugging it back in?".to_string());
                                let _ = result_tx.send(FlashOutcome::Failure(e.to_string()));
                                let _ = status_tx.send(AppStatus::Failure(e.to_string()));
                            }
                        }
                    }
                    Err(e) => {
                        let _ = ot_tx.send(e.to_string());
                        let _ = lp_tx.send(0.0);
                        ctx.request_repaint();
                        let _ = sleep(Duration::from_secs(10)).await;
                        let _ = ot_tx.send(
                            "Flashing failed... Did you try unplugging it and plugging it back in?"
                                .to_string(),
                        );
                        let _ = result_tx.send(FlashOutcome::Failure(e.to_string()));
                        let _ = status_tx.send(AppStatus::Failure(e.to_string()));
                    }
                }
            } else {
                let _ = ot_tx.send("Connecting to Target...".to_string());
                ctx.request_repaint();
                match flash.trigger_bootloader_via_usb() {
                    Ok(_sp) => {
                        let _ = ot_tx.send(
                            format!("USB connection initialized on {:?}", flash.selected_port)
                                .to_string(),
                        );
                        ctx.request_repaint();
                        match flash.connect_to_bootloader(5) {
                            Ok(_) => {
                                let _ = ot_tx.send(
                                    format!("Flashing to chip on {:?}", flash.selected_port)
                                        .to_string(),
                                );
                                ctx.request_repaint();
                                let detected_hw_rev = if flash.chip == ChipModel::ATSAMD21G18A {
                                    match flash.query_bootloader_hw_rev() {
                                        Ok(hw_rev) => {
                                            let _ = ot_tx.send(format!(
                                                "Detected board hardware revision: {}",
                                                format_hw_rev(hw_rev)
                                            ));
                                            hw_rev
                                        }
                                        Err(e) => {
                                            let _ = ot_tx.send(format!(
                                                "Unable to read board hardware revision: {e:#}"
                                            ));
                                            None
                                        }
                                    }
                                } else {
                                    None
                                };

                                let current_path = PathBuf::from(&path);
                                let flash_path = match resolve_firmware_for_detected_board(
                                    &update_identity,
                                    update_token.as_deref(),
                                    detected_hw_rev,
                                    &current_path,
                                    &ot_tx,
                                    &ui_tx,
                                ) {
                                    Ok(Some(resolved)) => resolved,
                                    Ok(None) => current_path,
                                    Err(e) => {
                                        let _ = ot_tx.send(format!(
                                            "Unable to resolve firmware for board {}: {e:#}",
                                            format_hw_rev(detected_hw_rev)
                                        ));
                                        current_path
                                    }
                                };

                                let ot_tx_evt = ot_tx.clone();
                                match flash
                                    .flash_via_usb(flash_path.display().to_string(), move |evt| {
                                        let _ = ot_tx_evt.send(format!("{:#?}", evt));
                                        ctx.request_repaint();
                                    })
                                    .await
                                {
                                    Ok(_) => {
                                        let _ = result_tx.send(FlashOutcome::Success);
                                        let _ = status_tx.send(AppStatus::Success);
                                    }
                                    Err(e) => {
                                        let msg = format!("Flashing failed: {}", e);
                                        let _ = ot_tx.send(msg.clone());
                                        let _ = result_tx.send(FlashOutcome::Failure(msg.clone()));
                                        let _ = status_tx.send(AppStatus::Failure(msg));
                                    }
                                }
                            }
                            Err(_e) => {
                                let _ = ot_tx.send(format!(
                                    "Could not find bootloader serial port; attempting flash on {}",
                                    flash.selected_port
                                ));
                                ctx.request_repaint();
                                println!("{}", path);
                                println!("{:#?}", flash.selected_port);
                                let detected_hw_rev = if flash.chip == ChipModel::ATSAMD21G18A {
                                    match flash.query_bootloader_hw_rev() {
                                        Ok(hw_rev) => {
                                            let _ = ot_tx.send(format!(
                                                "Detected board hardware revision: {}",
                                                format_hw_rev(hw_rev)
                                            ));
                                            hw_rev
                                        }
                                        Err(e) => {
                                            let _ = ot_tx.send(format!(
                                                "Unable to read board hardware revision: {e:#}"
                                            ));
                                            None
                                        }
                                    }
                                } else {
                                    None
                                };

                                let current_path = PathBuf::from(&path);
                                let flash_path = match resolve_firmware_for_detected_board(
                                    &update_identity,
                                    update_token.as_deref(),
                                    detected_hw_rev,
                                    &current_path,
                                    &ot_tx,
                                    &ui_tx,
                                ) {
                                    Ok(Some(resolved)) => resolved,
                                    Ok(None) => current_path,
                                    Err(e) => {
                                        let _ = ot_tx.send(format!(
                                            "Unable to resolve firmware for board {}: {e:#}",
                                            format_hw_rev(detected_hw_rev)
                                        ));
                                        current_path
                                    }
                                };

                                let ot_tx_evt = ot_tx.clone();
                                match flash
                                    .flash_via_usb(flash_path.display().to_string(), move |evt| {
                                        let _ = ot_tx_evt.send(format!("{:#?}", evt));
                                        ctx.request_repaint();
                                    })
                                    .await
                                {
                                    Ok(_) => {
                                        let _ = result_tx.send(FlashOutcome::Success);
                                        let _ = status_tx.send(AppStatus::Success);
                                    }
                                    Err(e) => {
                                        let msg = format!("Flashing failed: {}", e);
                                        let _ = ot_tx.send(msg.clone());
                                        let _ = result_tx.send(FlashOutcome::Failure(msg.clone()));
                                        let _ = status_tx.send(AppStatus::Failure(msg));
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = ot_tx
                            .send(format!("Error connecting to usb port: {:?}", e.to_string()));
                        ctx.request_repaint();
                        let _ = result_tx.send(FlashOutcome::Failure(e.to_string()));
                        let _ = status_tx.send(AppStatus::Failure(e.to_string()));
                    }
                };
            }
        });
    }

    /// Publish pending flash result after console output has been quiet briefly.
    fn maybe_publish_result(&mut self, now: Instant) {
        if self.flash_result.is_none()
            && let Some(outcome) = self.pending_result.clone()
            && now.duration_since(self.last_output) > Duration::from_millis(300)
        {
            self.flash_result = Some(outcome);
            self.pending_result = None;
        }
    }
}

impl App for FWUpdater {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui_extras::install_image_loaders(ctx);
        apply_companion_style(ctx);
        self.fd.lock().unwrap().update(ctx);

        if let Ok(response) = self.lp_rx.try_recv() {
            *self.load_progress.clone().lock().unwrap() = response;
        }
        if let Ok(response) = self.stat_rx.try_recv() {
            *self.state.clone().lock().unwrap() = response;
        }
        if let Ok(response) = self.ot_rx.try_recv() {
            self.console.write(response.as_str());
            self.last_output = Instant::now();
            if matches!(*self.state.lock().unwrap(), ViewState::Splash) {
                self.splash_status = response;
            }
        }
        if let Ok(response) = self.result_rx.try_recv() {
            self.pending_result = Some(response);
        }
        if let Ok(response) = self.status_rx.try_recv() {
            self.status = response;
        }
        if let Ok(state) = self.update_rx.try_recv() {
            self.update_state = state;
        }
        if let Ok(evt) = self.ui_rx.try_recv() {
            match evt {
                UiEvent::SetFirmwarePath(path) => {
                    self.elf_path = Arc::new(Mutex::new(Some(path.clone())));
                    self.fd_text = path.display().to_string();
                    self.console.write("Firmware downloaded and selected.");
                }
                UiEvent::UpdateResultAvailable(result) => {
                    self.last_update_result = Some(result);
                }
                UiEvent::FirmwareEnsureFinished => {
                    self.splash_firmware_in_flight = false;
                    self.splash_firmware_done = true;
                }
                UiEvent::BootstrapComplete(result) => {
                    self.device_token = Some(result.device_token);
                    if let Some(slug) = result.app_slug {
                        self.update_identity.slug = slug;
                    }
                    let runtime_os = std::env::consts::OS;
                    let runtime_arch = std::env::consts::ARCH;
                    if let Some(os) = result.os.filter(|s| !s.trim().is_empty()) {
                        if os == runtime_os {
                            self.update_identity.os = os;
                        } else {
                            self.console.write(&format!(
                                "Bootstrap returned os={os}, but this build is {runtime_os}; ignoring."
                            ));
                        }
                    }
                    if let Some(arch) = result.arch.filter(|s| !s.trim().is_empty()) {
                        if arch == runtime_arch {
                            self.update_identity.arch = arch;
                        } else {
                            self.console.write(&format!(
                                "Bootstrap returned arch={arch}, but this build is {runtime_arch}; ignoring."
                            ));
                        }
                    }
                    self.update_identity.bootstrap_pkg_url = None;
                    self.bootstrap_in_flight = false;
                    self.bootstrap_retry_at = None;
                    self.bootstrap_failures = 0;
                    self.update_check_started = false;
                    self.update_state = UpdateCheckState::Checking;
                    self.console.write("Device token acquired.");
                }
                UiEvent::BootstrapFailed(msg) => {
                    self.bootstrap_in_flight = false;
                    self.update_check_started = false;

                    let delay = next_bootstrap_backoff(self.bootstrap_failures);
                    self.bootstrap_failures = self.bootstrap_failures.saturating_add(1);
                    self.bootstrap_retry_at = Some(Instant::now() + delay);

                    let retry_msg = format!(
                        "Bootstrap exchange failed; retrying in {}s.",
                        delay.as_secs()
                    );
                    self.console.write(&retry_msg);
                    self.update_state = UpdateCheckState::Error(msg);
                }
                UiEvent::ExitRequested => {
                    self.exit_requested = true;
                }
            }
        }

        if self.exit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Delay showing the result popup until console output has settled briefly
        self.maybe_publish_result(Instant::now());

        match *self.state.clone().lock().unwrap() {
            ViewState::Splash => {
                CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(companion_bg()))
                    .show(ctx, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(96.0);
                            companion_panel_frame().show(ui, |ui| {
                                ui.set_width(520.0);
                                ui.vertical_centered(|ui| {
                                    ui.add(
                                        egui::Image::new(egui::include_image!(
                                            "../assets/pdes_tx_logo.png"
                                        ))
                                        .fit_to_original_size(1.0)
                                        .max_size(Vec2::new(360.0, 96.0)),
                                    );
                                    ui.add_space(18.0);
                                    ui.label(
                                        RichText::new("Firmware Updater")
                                            .size(22.0)
                                            .strong()
                                            .color(companion_text()),
                                    );
                                    ui.label(
                                        RichText::new(&self.splash_status)
                                            .size(14.0)
                                            .color(companion_muted()),
                                    );
                                    ui.add_space(16.0);
                                    self.update_splash(ctx);
                                    ui.add(
                                        egui::ProgressBar::new(*self.load_progress.lock().unwrap())
                                            .show_percentage()
                                            .animate(true)
                                            .fill(companion_accent_strong()),
                                    );
                                });
                            });
                        });
                    });
            }
            ViewState::Main => {
                self.ensure_preflight();
                TopBottomPanel::top("controls_panel")
                    .frame(
                        egui::Frame::NONE
                            .fill(companion_bg())
                            .inner_margin(egui::Margin::symmetric(20, 16)),
                    )
                    .resizable(true)
                    .default_height(UI_CONTROLS_DEFAULT_HEIGHT)
                    .min_height(UI_CONTROLS_MIN_HEIGHT)
                    .show(ctx, |ui| {
                        ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                let avail_rect = ui.available_rect_before_wrap();
                                let width = avail_rect.width().min(UI_CENTER_COLUMN_MAX_WIDTH);
                                let column_rect = egui::Rect::from_center_size(
                                    avail_rect.center(),
                                    Vec2::new(width, avail_rect.height()),
                                );
                                ui.scope_builder(
                                    egui::UiBuilder::new().max_rect(column_rect),
                                    |ui| {
                                        ui.set_width(width);

                                        ui.vertical_centered(|ui| {
                                            companion_panel_frame().show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.add(
                                                        egui::Image::new(egui::include_image!(
                                                            "../assets/pdes_tx_logo.png"
                                                        ))
                                                        .fit_to_original_size(1.0)
                                                        .max_size(Vec2::new(
                                                            width * 0.45,
                                                            UI_LOGO_MAX_HEIGHT,
                                                        )),
                                                    );
                                                    ui.with_layout(
                                                        egui::Layout::right_to_left(
                                                            egui::Align::Center,
                                                        ),
                                                        |ui| {
                                                            let (text, color) = match &self.status {
                                                                AppStatus::Idle => {
                                                                    ("Ready", companion_accent())
                                                                }
                                                                AppStatus::InProgress => (
                                                                    "Flashing",
                                                                    companion_accent_strong(),
                                                                ),
                                                                AppStatus::Success => (
                                                                    "Complete",
                                                                    companion_success(),
                                                                ),
                                                                AppStatus::Failure(_) => (
                                                                    "Needs attention",
                                                                    companion_warn(),
                                                                ),
                                                            };
                                                            status_badge(ui, text, color);
                                                            ui.vertical(|ui| {
                                                                ui.label(
                                                                    RichText::new(
                                                                        "Firmware Updater",
                                                                    )
                                                                    .size(21.0)
                                                                    .strong(),
                                                                );
                                                                ui.label(
                                                                    RichText::new(
                                                                        "PDES device firmware",
                                                                    )
                                                                    .size(13.0)
                                                                    .color(companion_muted()),
                                                                );
                                                            });
                                                        },
                                                    );
                                                });
                                            });
                                        });
                                        ui.add_space(12.0);

                                        companion_panel_frame().show(ui, |ui| {
                                            egui::CollapsingHeader::new(
                                                RichText::new("Pre-flight").strong(),
                                            )
                                            .default_open(self.preflight_open)
                                            .show(
                                                ui,
                                                |ui| {
                                                    ui.horizontal_wrapped(|ui| {
                                                        let (label, color) = if self
                                                            .preflight
                                                            .bossac_bundled
                                                        {
                                                            ("OK", companion_success())
                                                        } else if self.preflight.bossac_from_path {
                                                            ("PATH", companion_accent())
                                                        } else {
                                                            ("MISSING", companion_warn())
                                                        };
                                                        ui.label(
                                                            RichText::new("BOSSAC:")
                                                                .color(companion_muted()),
                                                        );
                                                        status_badge(ui, label, color);
                                                        ui.monospace(&self.preflight.bossac_path);
                                                    });

                                                    ui.horizontal_wrapped(|ui| {
                                                        ui.label(
                                                            RichText::new("USB ports:")
                                                                .color(companion_muted()),
                                                        );
                                                        ui.monospace(
                                                            self.preflight
                                                                .serial_ports
                                                                .len()
                                                                .to_string(),
                                                        );
                                                        if let Some(err) =
                                                            &self.preflight.serial_error
                                                        {
                                                            ui.colored_label(
                                                                companion_accent(),
                                                                err,
                                                            );
                                                        }
                                                    });

                                                    ui.horizontal_wrapped(|ui| {
                                                        ui.label(
                                                            RichText::new("Probes:")
                                                                .color(companion_muted()),
                                                        );
                                                        ui.monospace(
                                                            self.preflight.probes.len().to_string(),
                                                        );
                                                        if let Some(err) =
                                                            &self.preflight.probe_error
                                                        {
                                                            ui.colored_label(
                                                                companion_accent(),
                                                                err,
                                                            );
                                                        }
                                                    });

                                                    ui.horizontal(|ui| {
                                                        if ui
                                                            .add(
                                                                Button::new("Refresh")
                                                                    .fill(companion_input())
                                                                    .stroke(Stroke::new(
                                                                        1.0_f32,
                                                                        companion_border(),
                                                                    )),
                                                            )
                                                            .clicked()
                                                        {
                                                            self.flasher
                                                                .lock()
                                                                .unwrap()
                                                                .refresh_devices();
                                                            self.refresh_preflight();
                                                        }
                                                        if ui
                                                            .add(
                                                                Button::new("Updater token")
                                                                    .fill(companion_input())
                                                                    .stroke(Stroke::new(
                                                                        1.0_f32,
                                                                        companion_border(),
                                                                    )),
                                                            )
                                                            .clicked()
                                                        {
                                                            self.token_prompt_open = true;
                                                            self.token_prompt_dismissed = false;
                                                        }
                                                    });

                                                    ui.collapsing("Details", |ui| {
                                                        if !self.preflight.serial_ports.is_empty() {
                                                            ui.label("Serial ports:");
                                                            for p in &self.preflight.serial_ports {
                                                                ui.monospace(p);
                                                            }
                                                            ui.add_space(5.0);
                                                        }
                                                        if !self.preflight.probes.is_empty() {
                                                            ui.label("Debug probes:");
                                                            for p in &self.preflight.probes {
                                                                ui.monospace(p);
                                                            }
                                                        }
                                                    });
                                                },
                                            );
                                        });

                                        ui.add_space(12.0);
                                        companion_panel_frame().show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.add(egui::Label::new("Target: "));
                                                let mut flasher = self.flasher.lock().unwrap();
                                                egui::ComboBox::from_label("")
                                                    .selected_text(format!("{:?}", flasher.chip))
                                                    .width(ui.available_width() / 3.0)
                                                    .show_ui(ui, |ui| {
                                                        ui.selectable_value(
                                                            &mut flasher.chip,
                                                            ChipModel::ATSAMD21G18A,
                                                            "ATSAMD21",
                                                        );
                                                        ui.selectable_value(
                                                            &mut flasher.chip,
                                                            ChipModel::STM32F103C8,
                                                            "STM32F2",
                                                        );
                                                    });
                                            });

                                            ui.add_space(5.0);

                                            ui.horizontal(|ui| {
                                                ui.add(egui::Label::new("Method: "));
                                                let mut flasher = self.flasher.lock().unwrap();
                                                let selected_method = match flasher.flash_vector {
                                                    FlashVector::Usb => "USB (COM)",
                                                    FlashVector::Probe => "Probe (CMSIS-DAP)",
                                                };
                                                egui::ComboBox::from_id_salt("flash_method")
                                                    .selected_text(selected_method)
                                                    .width(ui.available_width() / 3.0)
                                                    .show_ui(ui, |ui| {
                                                        ui.selectable_value(
                                                            &mut flasher.flash_vector,
                                                            FlashVector::Usb,
                                                            "USB (COM)",
                                                        );
                                                        ui.selectable_value(
                                                            &mut flasher.flash_vector,
                                                            FlashVector::Probe,
                                                            "Probe (CMSIS-DAP)",
                                                        );
                                                    });
                                            });

                                            ui.add_space(5.0);

                                            {
                                                let mut flasher = self.flasher.lock().unwrap();
                                                match flasher.flash_vector {
                                                    FlashVector::Usb => {
                                                        ui.horizontal(|ui| {
                                                            ui.add(egui::Label::new("Port: "));
                                                            let selected_port_label = if flasher
                                                                .selected_port
                                                                .is_empty()
                                                            {
                                                                if cfg!(windows) {
                                                                    "No COM ports found".to_string()
                                                                } else {
                                                                    "No serial ports found"
                                                                        .to_string()
                                                                }
                                                            } else {
                                                                flasher.selected_port.clone()
                                                            };

                                                            let port_keys: Vec<String> = flasher
                                                                .ports
                                                                .keys()
                                                                .cloned()
                                                                .collect();
                                                            egui::ComboBox::from_id_salt(
                                                                "serial_port",
                                                            )
                                                            .selected_text(selected_port_label)
                                                            .width(ui.available_width() / 3.0)
                                                            .show_ui(ui, |ui| {
                                                                for k in port_keys {
                                                                    ui.selectable_value(
                                                                        &mut flasher.selected_port,
                                                                        k.clone(),
                                                                        k,
                                                                    );
                                                                }
                                                            });
                                                        });
                                                    }
                                                    FlashVector::Probe => {
                                                        ui.horizontal(|ui| {
                                                            ui.add(egui::Label::new("Probe: "));
                                                            let probe_labels: Vec<String> = flasher
                                                                .probes
                                                                .iter()
                                                                .map(|p| p.identifier.clone())
                                                                .collect();
                                                            let selected = probe_labels
                                                                .get(flasher.selected_probe_index)
                                                                .cloned()
                                                                .unwrap_or_else(|| {
                                                                    "No probe detected".to_string()
                                                                });
                                                            egui::ComboBox::from_id_salt("probe")
                                                                .selected_text(selected)
                                                                .width(ui.available_width() / 2.0)
                                                                .show_ui(ui, |ui| {
                                                                    for (idx, label) in probe_labels
                                                                        .iter()
                                                                        .enumerate()
                                                                    {
                                                                        ui.selectable_value(
                                                                        &mut flasher
                                                                            .selected_probe_index,
                                                                        idx,
                                                                        label,
                                                                    );
                                                                    }
                                                                });
                                                        });
                                                    }
                                                }
                                            }

                                            if ui
                                                .add(
                                                    egui::TextEdit::singleline(&mut self.fd_text)
                                                        .clip_text(false),
                                                )
                                                .clicked()
                                            {
                                                self.fd.lock().unwrap().pick_file();
                                            };
                                            // Firmware is auto-cached and selected on startup.
                                        });
                                    },
                                );
                            });
                    });

                TopBottomPanel::bottom("flash_panel")
                    .frame(
                        egui::Frame::NONE
                            .fill(companion_bg())
                            .inner_margin(egui::Margin::symmetric(20, 10)),
                    )
                    .exact_height(UI_FLASH_PANEL_HEIGHT)
                    .show(ctx, |ui| {
                        let button_size = UI_FLASH_BUTTON_SIZE;
                        ui.vertical_centered(|ui| {
                            let button = Button::new(
                                RichText::new("FLASH")
                                    .color(companion_bg())
                                    .strong()
                                    .size(16.0),
                            )
                            .fill(companion_accent_strong())
                            .stroke(Stroke::new(1.0_f32, companion_accent()));
                            if ui.add_sized(button_size, button).clicked() {
                                let path = self
                                    .elf_path
                                    .lock()
                                    .unwrap()
                                    .as_ref()
                                    .map(|p| p.display().to_string());
                                let Some(path) = path else {
                                    self.console.write("Select a firmware file first.");
                                    return;
                                };

                                self.show_output = !self.show_output;
                                let flasher: Flasher = self.flasher.lock().unwrap().clone();
                                self.start_flash_op(flasher, path, ctx.clone());
                            }
                            ui.add_space(2.0);
                            let version_color = match &self.update_state {
                                UpdateCheckState::UpToDate => companion_accent(),
                                UpdateCheckState::UpdateAvailable(result) => {
                                    let current_version = env!("CARGO_PKG_VERSION");
                                    let app_update = result.app.as_ref().is_some_and(|a| {
                                        updater::is_newer_version(
                                            current_version,
                                            &a.latest_version,
                                        )
                                    });
                                    if app_update {
                                        companion_muted()
                                    } else {
                                        companion_accent()
                                    }
                                }
                                UpdateCheckState::Error(_) => companion_warn(),
                                _ => companion_muted(),
                            };

                            let fw_version = self
                                .last_update_result
                                .as_ref()
                                .and_then(|r| r.firmware.as_ref())
                                .map(|f| f.latest_version.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| "—".to_string());

                            ui.horizontal(|ui| {
                                let half = ui.available_width() / 2.0;

                                ui.allocate_ui_with_layout(
                                    Vec2::new(half, 0.0),
                                    egui::Layout::left_to_right(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new("App Version:")
                                                .size(12.0)
                                                .color(companion_muted()),
                                        );
                                        ui.label(
                                            RichText::new(format!("v{}", self.app_version))
                                                .size(12.0)
                                                .color(version_color),
                                        );
                                    },
                                );

                                ui.allocate_ui_with_layout(
                                    Vec2::new(half, 0.0),
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format!("v{fw_version}"))
                                                .size(12.0)
                                                .color(version_color),
                                        );
                                        ui.label(
                                            RichText::new("Firmware Version:")
                                                .size(12.0)
                                                .color(companion_muted()),
                                        );
                                    },
                                );
                            });
                        });
                    });

                CentralPanel::default()
                    .frame(
                        egui::Frame::NONE
                            .fill(companion_bg())
                            .inner_margin(egui::Margin::symmetric(20, 8)),
                    )
                    .show(ctx, |ui| {
                        companion_soft_frame().show(ui, |ui| {
                            ui.label(
                                RichText::new("Output")
                                    .size(13.0)
                                    .strong()
                                    .color(companion_muted()),
                            );
                            ui.add_space(4.0);
                            self.console.draw(ui);
                        });
                    });

                if let Some(path) = self.fd.lock().unwrap().take_picked() {
                    self.elf_path = Arc::new(Mutex::new(Some(path.to_path_buf())));
                    self.fd_text = path.to_str().unwrap().to_string();
                }
            }
        }

        if let Some(outcome) = &self.flash_result {
            let mut keep_open = true;
            let mut close_now = false;
            let (title, msg, color) = match outcome {
                FlashOutcome::Success => (
                    "Flash succeeded",
                    "Firmware flashing completed successfully.",
                    companion_success(),
                ),
                FlashOutcome::Failure(err) => ("Flash failed", err.as_str(), companion_warn()),
            };
            egui::Window::new(title)
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .collapsible(false)
                .resizable(false)
                .auto_sized()
                .open(&mut keep_open)
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.colored_label(color, msg);
                        ui.add_space(10.0);
                        if ui
                            .add(
                                Button::new(RichText::new("Close").color(companion_bg()).strong())
                                    .fill(companion_accent_strong())
                                    .stroke(Stroke::new(1.0_f32, companion_accent())),
                            )
                            .clicked()
                        {
                            close_now = true;
                        }
                    });
                });
            if close_now || !keep_open {
                self.flash_result = None;
            }
        }

        if self.token_prompt_open {
            let mut keep_open = true;
            egui::Window::new("Updater setup")
                .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
                .collapsible(false)
                .resizable(false)
                .auto_sized()
                .open(&mut keep_open)
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new(
                            "Enter your updater token. This is provided on the download page.",
                        )
                        .color(companion_muted()),
                    );
                    ui.add_space(5.0);

                    ui.horizontal(|ui| {
                        ui.label("Token:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.token_input)
                                .password(true)
                                .desired_width(260.0),
                        );
                    });

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                Button::new(RichText::new("Save").color(companion_bg()).strong())
                                    .fill(companion_accent_strong())
                                    .stroke(Stroke::new(1.0_f32, companion_accent())),
                            )
                            .clicked()
                        {
                            let token = self.token_input.trim().to_string();
                            self.settings.update_token = (!token.is_empty()).then_some(token);

                            match self.settings.save() {
                                Ok(_) => {
                                    self.console.write("Updater settings saved.");
                                    self.token_prompt_open = false;
                                    self.token_prompt_dismissed = false;
                                    self.update_check_started = false;
                                    self.ensure_updater(false);
                                }
                                Err(e) => {
                                    self.console.write(
                                        format!("Failed to save updater settings: {e}").as_str(),
                                    );
                                }
                            }
                        }

                        if ui
                            .add(
                                Button::new("Later")
                                    .fill(companion_input())
                                    .stroke(Stroke::new(1.0_f32, companion_border())),
                            )
                            .clicked()
                        {
                            self.token_prompt_open = false;
                            self.token_prompt_dismissed = true;
                        }
                    });
                });
            if !keep_open {
                self.token_prompt_open = false;
                self.token_prompt_dismissed = true;
            }
        }
    }
}

fn main() -> eframe::Result<()> {
    let rt = Runtime::new().expect("Unable to create Runtime");

    let _enter = rt.enter();

    std::thread::spawn(move || {
        rt.block_on(async {
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        })
    });

    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id(LINUX_DESKTOP_APP_ID)
            .with_inner_size([1024.0, 840.0])
            .with_min_inner_size([900.0, 720.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Palo Duro Embedded Systems Firmware Updater",
        options,
        Box::new(|cc| Ok(Box::new(FWUpdater::new(cc)))),
    )
}

fn format_serial_port(port: &SerialPortInfo) -> String {
    let mut out = port.port_name.clone();
    match &port.port_type {
        SerialPortType::UsbPort(info) => {
            out.push_str(&format!(" (USB {:04x}:{:04x}", info.vid, info.pid));
            if let Some(product) = &info.product {
                out.push_str(&format!(", {}", product));
            }
            if let Some(manufacturer) = &info.manufacturer {
                out.push_str(&format!(", {}", manufacturer));
            }
            out.push(')');
        }
        SerialPortType::PciPort => out.push_str(" (PCI)"),
        SerialPortType::BluetoothPort => out.push_str(" (Bluetooth)"),
        SerialPortType::Unknown => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{FWUpdater, FlashOutcome};
    use std::time::{Duration, Instant};

    #[test]
    fn pending_result_waits_for_quiet_console() {
        let mut app = FWUpdater {
            pending_result: Some(FlashOutcome::Success),
            last_output: Instant::now(),
            ..Default::default()
        };

        let now = app.last_output + Duration::from_millis(200);
        app.maybe_publish_result(now);

        assert!(app.flash_result.is_none());
        assert!(app.pending_result.is_some());
    }

    #[test]
    fn pending_result_publishes_after_quiet_period() {
        let mut app = FWUpdater {
            pending_result: Some(FlashOutcome::Success),
            last_output: Instant::now() - Duration::from_millis(400),
            ..Default::default()
        };

        let now = Instant::now();
        app.maybe_publish_result(now);

        assert!(app.flash_result.is_some());
        assert!(app.pending_result.is_none());
    }
}
