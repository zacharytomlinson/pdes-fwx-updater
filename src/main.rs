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

use crate::downloader::{download_app_pkg_to_staging_verified, download_to_app_firmware_verified};
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
use std::path::PathBuf;
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
}

const UI_CENTER_COLUMN_MAX_WIDTH: f32 = 900.0;
const UI_CONTROLS_DEFAULT_HEIGHT: f32 = 420.0;
const UI_CONTROLS_MIN_HEIGHT: f32 = 220.0;
const UI_LOGO_MAX_HEIGHT: f32 = 80.0;
const UI_FLASH_PANEL_HEIGHT: f32 = 54.0;
const UI_FLASH_BUTTON_SIZE: Vec2 = Vec2::new(140.0, 32.0);

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

    update_state: UpdateCheckState,
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
        let update_identity = UpdateIdentity::load_or_default();
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
            update_state: UpdateCheckState::NotConfigured,
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

        if let Err(e) = crate::app_dirs::ensure_dirs() {
            let msg = format!("Unable to initialize persistent storage: {e:#}");
            app.console.write(&msg);
        }

        match crate::seed::ensure_seed_config() {
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

    fn ensure_updater(&mut self) {
        let token = self.effective_update_token();
        if token.is_none() {
            if self.maybe_start_bootstrap_exchange() {
                return;
            }
            self.update_state = UpdateCheckState::NotConfigured;
            if !self.token_prompt_dismissed {
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
        let firmware_url = self.update_identity.firmware_latest_url();

        let tx = self.update_tx.clone();
        let current_version = env!("CARGO_PKG_VERSION").to_string();
        std::thread::spawn(move || {
            let _ = tx.send(UpdateCheckState::Checking);
            match updater::check_for_updates(&package_url, &firmware_url, &token) {
                Ok(result) => {
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
                    let _ = tx.send(UpdateCheckState::Error(e.to_string()));
                }
            }
        });
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

    // Simulate loading or perform actual loading tasks here
    /// Kick off installer/bootstrap work while updating the splash progress bar.
    fn update_progress_bar(
        &mut self,
        stat_tx: Sender<ViewState>,
        lp_tx: Sender<f32>,
        ctx: egui::Context,
    ) {
        let prog = *self.load_progress.clone().lock().unwrap();
        if prog < 1.0 {
            let new_prog = (prog + 0.05).min(1.0);
            let _ = lp_tx.send(new_prog);
            if new_prog >= 1.0 {
                let _ = stat_tx.send(ViewState::Main);
            }
            ctx.request_repaint();
        } else {
            let _ = lp_tx.send(0.0);
            let _ = stat_tx.send(ViewState::Main); // Transition to main app
            ctx.request_repaint();
        }
    }

    /// Start flashing operation, dispatching to probe or USB depending on flash vector.
    fn start_flash_op(&mut self, mut flash: Flasher, path: String, ctx: egui::Context) {
        let prog = *self.load_progress.clone().lock().unwrap();
        let ot_tx = self.ot_tx.clone();
        let lp_tx = self.lp_tx.clone();
        let result_tx = self.result_tx.clone();
        let status_tx = self.status_tx.clone();
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
                                let ot_tx_evt = ot_tx.clone();
                                match flash
                                    .flash_via_usb(path, move |evt| {
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
                                let ot_tx_evt = ot_tx.clone();
                                match flash
                                    .flash_via_usb(path, move |evt| {
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
                UiEvent::BootstrapComplete(result) => {
                    self.device_token = Some(result.device_token);
                    if let Some(slug) = result.app_slug {
                        self.update_identity.slug = slug;
                    }
                    if let Some(os) = result.os {
                        self.update_identity.os = os;
                    }
                    if let Some(arch) = result.arch {
                        self.update_identity.arch = arch;
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
            }
        }

        // Delay showing the result popup until console output has settled briefly
        self.maybe_publish_result(Instant::now());

        match *self.state.clone().lock().unwrap() {
            ViewState::Splash => {
                CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(100.0);
                        ui.add(egui::Image::new(egui::include_image!(
                            "../assets/pdes_tx_logo.png"
                        )));
                        ui.add_space(20.0);
                        ui.add(
                            egui::ProgressBar::new(*self.load_progress.lock().unwrap())
                                .show_percentage(),
                        );
                        ui.add_space(20.0);
                    });

                    self.update_progress_bar(self.stat_tx.clone(), self.lp_tx.clone(), ctx.clone());
                });
            }
            ViewState::Main => {
                self.ensure_preflight();
                self.ensure_updater();
                TopBottomPanel::top("controls_panel")
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
                                ui.scope_builder(egui::UiBuilder::new().max_rect(column_rect), |ui| {
                                    ui.set_width(width);

                                    ui.vertical_centered(|ui| {
                                        ui.add(
                                            egui::Image::new(egui::include_image!(
                                                "../assets/pdes_tx_logo.png"
                                            ))
                                            .fit_to_original_size(1.0)
                                            .max_size(Vec2::new(width, UI_LOGO_MAX_HEIGHT)),
                                        );
                                    });
                                    ui.add_space(15.0);

                                                egui::CollapsingHeader::new("Pre-flight")
                                                    .default_open(self.preflight_open)
                                                    .show(ui, |ui| {
                                                        ui.horizontal_wrapped(|ui| {
                                                            let (label, color) = if self
                                                                .preflight
                                                                .bossac_bundled
                                                            {
                                                                ("OK", Color32::GREEN)
                                                            } else if self.preflight.bossac_from_path {
                                                                ("PATH", Color32::YELLOW)
                                                            } else {
                                                                ("MISSING", Color32::RED)
                                                            };
                                                            ui.label("BOSSAC:");
                                                            ui.colored_label(color, label);
                                                            ui.monospace(&self.preflight.bossac_path);
                                                        });

                                        ui.horizontal_wrapped(|ui| {
                                            ui.label("USB ports:");
                                            ui.monospace(
                                                self.preflight.serial_ports.len().to_string(),
                                            );
                                            if let Some(err) = &self.preflight.serial_error {
                                                ui.colored_label(Color32::YELLOW, err);
                                            }
                                        });

                                        ui.horizontal_wrapped(|ui| {
                                            ui.label("Probes:");
                                            ui.monospace(self.preflight.probes.len().to_string());
                                            if let Some(err) = &self.preflight.probe_error {
                                                ui.colored_label(Color32::YELLOW, err);
                                            }
                                        });

                                        ui.horizontal(|ui| {
                                            if ui.button("Refresh").clicked() {
                                                self.flasher.lock().unwrap().refresh_devices();
                                                self.refresh_preflight();
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
                                    });

                                egui::CollapsingHeader::new("Updates")
                                    .default_open(false)
                                    .show(ui, |ui| {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label("Status:");
                                            match &self.update_state {
                                                UpdateCheckState::NotConfigured => {
                                                    ui.colored_label(
                                                        Color32::YELLOW,
                                                        "Token not set",
                                                    );
                                                }
                                                UpdateCheckState::Checking => {
                                                    ui.label("Checking...");
                                                }
                                                UpdateCheckState::UpToDate => {
                                                    ui.colored_label(Color32::GREEN, "Up to date");
                                                }
                                                UpdateCheckState::UpdateAvailable(result) => {
                                                    ui.colored_label(
                                                        Color32::YELLOW,
                                                        "Update available",
                                                    );
                                                    if let Some(app) = &result.app {
                                                        ui.separator();
                                                        ui.horizontal_wrapped(|ui| {
                                                            ui.label("App:");
                                                            ui.monospace(format!(
                                                                "{} → {}",
                                                                env!("CARGO_PKG_VERSION"),
                                                                app.latest_version
                                                            ));
                                                        });
                                                        if ui
                                                            .add_enabled(
                                                                app.download_url.is_some(),
                                                                egui::Button::new(
                                                                    "Install app update",
                                                                ),
                                                            )
                                                            .clicked()
                                                            && let Some(url) =
                                                                app.download_url.clone()
                                                        {
                                                            let Some(token) =
                                                                self.settings.update_token.clone()
                                                            else {
                                                                self.console
                                                                    .write("Updater token not set.");
                                                                return;
                                                            };
                                                            let Some(expected_sha) =
                                                                app.sha256.clone()
                                                            else {
                                                                self.console.write(
                                                                    "Update metadata missing pkg_sha256; cannot verify download.",
                                                                );
                                                                return;
                                                            };

                                                            let ot_tx = self.ot_tx.clone();
                                                            self.console.write(
                                                                "Downloading package...",
                                                            );
                                                            std::thread::spawn(move || {
                                                                let pkg_path = match download_app_pkg_to_staging_verified(
                                                                    &url,
                                                                    &token,
                                                                    &expected_sha,
                                                                ) {
                                                                    Ok(path) => path,
                                                                    Err(e) => {
                                                                        let _ = ot_tx.send(format!(
                                                                            "Package download failed: {e}"
                                                                        ));
                                                                        return;
                                                                    }
                                                                };
                                                                let _ = ot_tx.send(format!(
                                                                    "Package downloaded and verified: {}",
                                                                    pkg_path.display()
                                                                ));

                                                                let plan =
                                                                    match extract_pkg_and_find_installer(&pkg_path) {
                                                                        Ok(plan) => plan,
                                                                        Err(e) => {
                                                                            let _ = ot_tx.send(
                                                                                format!(
                                                                                    "Unable to extract package: {e}"
                                                                                ),
                                                                            );
                                                                            return;
                                                                        }
                                                                    };
                                                                let _ = ot_tx.send(format!(
                                                                    "Launching installer: {}",
                                                                    plan.installer_path.display()
                                                                ));
                                                                if let Err(e) = launch_installer(
                                                                    &plan.installer_path,
                                                                ) {
                                                                    let _ = ot_tx.send(format!(
                                                                        "Unable to launch installer: {e}"
                                                                    ));
                                                                } else {
                                                                    let _ = ot_tx.send(
                                                                        "Installer launched. Close and re-open PDES after the update completes."
                                                                            .to_string(),
                                                                    );
                                                                }
                                                            });
                                                        }
                                                        if let Some(notes) = &app.notes {
                                                            ui.collapsing("Notes", |ui| {
                                                                ui.label(notes);
                                                            });
                                                        }
                                                    }

                                                    if let Some(fw) = &result.firmware {
                                                        ui.separator();
                                                        ui.horizontal_wrapped(|ui| {
                                                            ui.label("Firmware:");
                                                            ui.monospace(&fw.latest_version);
                                                        });
                                                        if ui
                                                            .add_enabled(
                                                                fw.download_url.is_some(),
                                                                egui::Button::new(
                                                                    "Download firmware update",
                                                                ),
                                                            )
                                                            .clicked()
                                                            && let Some(url) =
                                                                fw.download_url.clone()
                                                        {
                                                            let Some(token) =
                                                                self.settings.update_token.clone()
                                                            else {
                                                                self.console
                                                                    .write("Updater token not set.");
                                                                return;
                                                            };
                                                            let Some(expected_sha) =
                                                                fw.sha256.clone()
                                                            else {
                                                                self.console.write(
                                                                    "Update metadata missing sha256; cannot verify download.",
                                                                );
                                                                return;
                                                            };

                                                            let ot_tx = self.ot_tx.clone();
                                                            let ui_tx = self.ui_tx.clone();
                                                            self.console.write(
                                                                "Downloading firmware...",
                                                            );
                                                            std::thread::spawn(move || {
                                                                match download_to_app_firmware_verified(
                                                                    &url,
                                                                    &token,
                                                                    &expected_sha,
                                                                ) {
                                                                    Ok(path) => {
                                                                        let _ = ot_tx.send(
                                                                            format!(
                                                                                "Downloaded firmware to {}",
                                                                                path.display()
                                                                            ),
                                                                        );
                                                                        let _ = ui_tx.send(
                                                                            UiEvent::SetFirmwarePath(
                                                                                path,
                                                                            ),
                                                                        );
                                                                    }
                                                                    Err(e) => {
                                                                        let _ = ot_tx.send(
                                                                            format!(
                                                                                "Firmware download failed: {e}"
                                                                            ),
                                                                        );
                                                                    }
                                                                }
                                                            });
                                                        }
                                                        if let Some(notes) = &fw.notes {
                                                            ui.collapsing("Notes", |ui| {
                                                                ui.label(notes);
                                                            });
                                                        }
                                                    }
                                                }
                                                UpdateCheckState::Error(e) => {
                                                    ui.colored_label(Color32::RED, e);
                                                }
                                            }
                                        });

                                        ui.horizontal(|ui| {
                                            if ui.button("Set token").clicked() {
                                                self.token_prompt_open = true;
                                                self.token_prompt_dismissed = false;
                                            }
                                            if ui.button("Check now").clicked() {
                                                self.update_check_started = false;
                                                self.bootstrap_retry_at = None;
                                                self.bootstrap_failures = 0;
                                                self.ensure_updater();
                                            }
                                        });
                                    });

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

                                let mut flasher = self.flasher.lock().unwrap();
                                match flasher.flash_vector {
                                    FlashVector::Usb => {
                                        ui.horizontal(|ui| {
                                            ui.add(egui::Label::new("Port: "));
                                            let selected_port_label =
                                                if flasher.selected_port.is_empty() {
                                                    if cfg!(windows) {
                                                        "No COM ports found".to_string()
                                                    } else {
                                                        "No serial ports found".to_string()
                                                    }
                                                } else {
                                                    flasher.selected_port.clone()
                                                };

                                            let port_keys: Vec<String> =
                                                flasher.ports.keys().cloned().collect();
                                            egui::ComboBox::from_id_salt("serial_port")
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
                                                .unwrap_or_else(|| "No probe detected".to_string());
                                            egui::ComboBox::from_id_salt("probe")
                                                .selected_text(selected)
                                                .width(ui.available_width() / 2.0)
                                                .show_ui(ui, |ui| {
                                                    for (idx, label) in
                                                        probe_labels.iter().enumerate()
                                                    {
                                                        ui.selectable_value(
                                                            &mut flasher.selected_probe_index,
                                                            idx,
                                                            label,
                                                        );
                                                    }
                                                });
                                        });
                                    }
                                }

                                                        if ui
                                                            .add(
                                                                egui::TextEdit::singleline(
                                                                    &mut self.fd_text,
                                                                )
                                                                .clip_text(false),
                                                            )
                                                                .clicked()
                                                        {
                                                            self.fd.lock().unwrap().pick_file();
                                                        };
                                });
                            });
                    });

                TopBottomPanel::bottom("flash_panel")
                    .exact_height(UI_FLASH_PANEL_HEIGHT)
                    .show(ctx, |ui| {
                        let avail_rect = ui.available_rect_before_wrap();
                        let button_size = UI_FLASH_BUTTON_SIZE;
                        let button_rect =
                            egui::Rect::from_center_size(avail_rect.center(), button_size);
                        ui.scope_builder(egui::UiBuilder::new().max_rect(button_rect), |ui| {
                            let button =
                                Button::new(RichText::new("FLASH").color(Color32::DARK_BLUE))
                                    .fill(Color32::GREEN)
                                    .stroke(Stroke::new(1.0, egui::Color32::DARK_BLUE));
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
                        });
                    });

                CentralPanel::default().show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(egui::Color32::DARK_GRAY)
                        .corner_radius(5)
                        .stroke(Stroke::new(1.0, egui::Color32::WHITE))
                        .inner_margin(egui::Margin::same(6))
                        .show(ui, |ui| {
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
                    Color32::GREEN,
                ),
                FlashOutcome::Failure(err) => ("Flash failed", err.as_str(), Color32::RED),
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
                        if ui.button("Close").clicked() {
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
                    ui.label("Enter your updater token. This is provided on the download page.");
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
                        if ui.button("Save").clicked() {
                            let token = self.token_input.trim().to_string();
                            self.settings.update_token = (!token.is_empty()).then_some(token);

                            match self.settings.save() {
                                Ok(_) => {
                                    self.console.write("Updater settings saved.");
                                    self.token_prompt_open = false;
                                    self.token_prompt_dismissed = false;
                                    self.update_check_started = false;
                                    self.ensure_updater();
                                }
                                Err(e) => {
                                    self.console.write(
                                        format!("Failed to save updater settings: {e}").as_str(),
                                    );
                                }
                            }
                        }

                        if ui.button("Later").clicked() {
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
            .with_inner_size([1024.0, 820.0])
            .with_min_inner_size([900.0, 700.0]),
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
