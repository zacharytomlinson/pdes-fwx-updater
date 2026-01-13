#![feature(trim_prefix_suffix)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod flasher;

use crate::flasher::{ChipModel, FlashVector};
use eframe::epaint::Color32;
use eframe::{App, CreationContext, NativeOptions, egui};
use egui::{Align2, Area, Button, CentralPanel, Id, RichText, Stroke, Vec2};
use egui_console::{ConsoleBuilder, ConsoleWindow};
use egui_file_dialog::FileDialog;
use flasher::Flasher;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::runtime::Runtime;
use tokio::time::sleep;

#[cfg(target_os = "linux")]
const LINUX_INSTALLER: &str = "./scripts/linux/pdes-linux-installer.sh";
#[cfg(target_os = "windows")]
const WIN_INSTALLER: &str = "./scripts/windows/pdes-windows-installer.ps1";

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
}

impl Default for FWUpdater {
    fn default() -> Self {
        let (lp_tx, lp_rx) = channel();
        let (stat_tx, stat_rx) = channel();
        let (ot_tx, ot_rx) = channel();
        let (result_tx, result_rx) = channel();
        let (status_tx, status_rx) = channel();
        Self {
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
        }
    }
}

impl FWUpdater {
    fn new(_cc: &CreationContext<'_>) -> Self {
        Default::default()
    }

    // Simulate loading or perform actual loading tasks here
    fn update_progress_bar(
        &mut self,
        stat_tx: Sender<ViewState>,
        lp_tx: Sender<f32>,
        ctx: egui::Context,
    ) {
        let prog = *self.load_progress.clone().lock().unwrap();
        tokio::spawn(async move {
            if prog < 1.0 {
                #[cfg(target_os = "windows")]
                {
                    Command::new("powershell")
                        .args([
                            "-NoProfile",
                            "-Command",
                            &format!(
                                "Start-Process powershell \
                 -ArgumentList '-NoProfile -ExecutionPolicy Bypass -File \"{}\"' \
                 -Verb RunAs",
                                WIN_INSTALLER
                            ),
                        ])
                        .spawn()
                        .expect("failed to run elevated installer");
                }

                #[cfg(target_os = "linux")]
                {
                    Command::new("chmod")
                        .arg("+x")
                        .arg(LINUX_INSTALLER)
                        .status()
                        .await
                        .expect("chmod failed");

                    Command::new("bash")
                        .arg(LINUX_INSTALLER)
                        .spawn()
                        .expect("failed to run linux installer");
                }

                let _ = lp_tx.send(prog + 0.05);
                ctx.request_repaint();
            } else {
                let _ = lp_tx.send(0.0);
                let _ = stat_tx.send(ViewState::Main); // Transition to main app
                ctx.request_repaint();
            }
        });
    }

    fn start_flash_op(
        &mut self,
        mut flash: Flasher,
        ot_tx: Sender<String>,
        lp_tx: Sender<f32>,
        result_tx: Sender<FlashOutcome>,
        status_tx: Sender<AppStatus>,
        ctx: egui::Context,
    ) {
        let prog = *self.load_progress.clone().lock().unwrap();
        let path = self
            .elf_path
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .display()
            .to_string();
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
                                ot_tx
                                    .send(
                                        format!(
                                            "Could not find serial port, defaulting to {:?}",
                                            flash.selected_port
                                        )
                                        .to_string(),
                                    )
                                    .expect("Shits all fucked");
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
                CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        Area::new(Id::from("top_center_area"))
                            .anchor(Align2::CENTER_TOP, [0.0, 15.0])
                            .default_height(ui.available_height() * 0.15)
                            .default_width(ui.available_width() * 0.9)
                            .show(ctx, |ui| {
                                ui.add(
                                    egui::Image::new(egui::include_image!(
                                        "../assets/pdes_tx_logo.png"
                                    ))
                                    .max_height(45.0),
                                );
                                ui.add_space(15.0);

                                ui.horizontal(|ui| {
                                    ui.add(egui::Label::new("Target: "));
                                    egui::ComboBox::from_label("")
                                        .selected_text(format!(
                                            "{:?}",
                                            self.flasher.lock().unwrap().chip.clone()
                                        ))
                                        .width(ui.available_width() / 3.0)
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut self.flasher.lock().unwrap().chip,
                                                ChipModel::ATSAMD21G18A,
                                                "ATSAMD21",
                                            );
                                            ui.selectable_value(
                                                &mut self.flasher.lock().unwrap().chip,
                                                ChipModel::STM32F103C8,
                                                "STM32F2",
                                            );
                                        });
                                });

                                ui.add_space(5.0);

                                ui.horizontal(|ui| {
                                    ui.add(egui::Label::new("Port: "));
                                    let selected_port_label = {
                                        let flasher = self.flasher.lock().unwrap();
                                        if flasher.selected_port.is_empty() {
                                            if cfg!(windows) {
                                                "No COM ports found".to_string()
                                            } else {
                                                "No serial ports found".to_string()
                                            }
                                        } else {
                                            flasher.selected_port.clone()
                                        }
                                    };
                                    egui::ComboBox::from_label(" ")
                                        .selected_text(selected_port_label)
                                        .width(ui.available_width() / 3.0)
                                        .show_ui(ui, |ui| {
                                            let mut flasher = self.flasher.lock().unwrap().clone();
                                            for k in flasher.ports.keys() {
                                                ui.selectable_value(
                                                    &mut flasher.selected_port,
                                                    k.clone(),
                                                    k,
                                                );
                                            }
                                        });
                                });

                                ui.add_space(5.0);

                                if ui
                                    .add(
                                        egui::TextEdit::singleline(&mut self.fd_text)
                                            .clip_text(false),
                                    )
                                    .clicked()
                                {
                                    self.fd.lock().unwrap().pick_file();
                                };
                            });

                        ui.add_space(15.0);

                        Area::new(Id::from("middle_center_area"))
                            .anchor(Align2::CENTER_CENTER, [5.0, 50.0])
                            .default_height(ui.available_height() * 0.70)
                            .default_width(ui.available_width() * 0.9)
                            .show(ctx, |ui| {
                                egui::Frame::NONE
                                    .fill(egui::Color32::DARK_GRAY)
                                    .corner_radius(5)
                                    .stroke(Stroke::new(1.0, egui::Color32::WHITE))
                                    .show(ui, |ui| {
                                        self.console.draw(ui);
                                    });
                            });

                        if let Some(path) = self.fd.lock().unwrap().take_picked() {
                            self.elf_path = Arc::new(Mutex::new(Some(path.to_path_buf())));
                            self.fd_text = path.to_str().unwrap().to_string();
                        }

                        ui.add_space(10.0);
                        Area::new(Id::from("bottom_center_area"))
                            .anchor(Align2::CENTER_BOTTOM, [0.0, -12.5])
                            .show(ui.ctx(), |ui| {
                                ui.horizontal_centered(|ui| {
                                    let button = Button::new(
                                        RichText::new("FLASH").color(Color32::DARK_BLUE),
                                    )
                                    .fill(Color32::GREEN)
                                    .stroke(Stroke::new(1.0, egui::Color32::DARK_BLUE));
                                    if ui.add_sized(Vec2::new(100.0, 25.0), button).clicked() {
                                        self.show_output = !self.show_output;
                                        let flasher: Flasher = self.flasher.lock().unwrap().clone();
                                        self.start_flash_op(
                                            flasher,
                                            self.ot_tx.clone(),
                                            self.lp_tx.clone(),
                                            self.result_tx.clone(),
                                            self.status_tx.clone(),
                                            ctx.clone(),
                                        );
                                    }
                                });
                            });
                    });
                });
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

    let options: NativeOptions = NativeOptions::default();
    eframe::run_native(
        "Palo Duro Embedded Systems Firmware Updater",
        options,
        Box::new(|cc| Ok(Box::new(FWUpdater::new(cc)))),
    )
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
