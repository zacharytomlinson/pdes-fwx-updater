#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod flasher;

use crate::flasher::ChipModel;
use eframe::epaint::Color32;
use eframe::{egui, App, CreationContext, NativeOptions};
use egui::{Align2, Area, Button, CentralPanel, Context, Id, RichText, Stroke, Vec2};
use egui_console::{ConsoleBuilder, ConsoleWindow};
use egui_file_dialog::FileDialog;
use flasher::Flasher;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::time::sleep;

#[derive(Default, Debug, Copy, Clone)]
pub enum ViewState {
    #[default]
    Splash,
    Main,
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
}

impl Default for FWUpdater {
    fn default() -> Self {
        let (lp_tx, lp_rx) = channel();
        let (stat_tx, stat_rx) = channel();
        let (ot_tx, ot_rx) = channel();
        Self {
            state: Arc::new(Mutex::new(ViewState::Splash)),
            load_progress: Arc::new(Mutex::new(0.0)),
            flasher: Default::default(),
            elf_path: Arc::new(Mutex::new(None)),
            fd: Default::default(),
            fd_text: "Select Binary to Flash".to_string(),
            show_output: false,
            console: ConsoleBuilder::new()
                .prompt("PDES >>")
                .history_size(100)
                .scrollback_size(10)
                .build(),

            lp_rx,
            lp_tx,
            stat_tx,
            stat_rx,
            ot_tx,
            ot_rx,
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
        ctx: Context,
    ) {
        let prog = *self.load_progress.clone().lock().unwrap();
        tokio::spawn(async move {
            if prog < 1.0 {
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
        flash: Flasher,
        ot_tx: Sender<String>,
        lp_tx: Sender<f32>,
        ctx: Context,
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
        tokio::spawn(async move {
            let _ = ot_tx.send("Connecting to Probe...".to_string());
            ctx.request_repaint();
            match flash.connect() {
                Ok(probe) => {
                    let _ = lp_tx.send(prog + 40.0);
                    let _ = ot_tx.send("Attaching Probe to Target...".to_string());
                    ctx.request_repaint();
                    match flash.attach(probe) {
                        Ok(session) => {
                            let _ = lp_tx.send(prog + 5.0);
                            let _ = ot_tx.send("Flashing Binary to Target".to_string());
                            ctx.request_repaint();
                            match flash.flash(session, path, |evt| {
                                let _ = ot_tx.send(format!("{:#?}", evt));
                                ctx.request_repaint();
                            }) {
                                Ok(_) => {
                                    let _ = lp_tx.send(prog + 5.0);
                                    let _ = ot_tx.send("Flashing Successful!".to_string());
                                    ctx.request_repaint();
                                }
                                Err(e) => {
                                    let _ = ot_tx.send(e.to_string());
                                    let _ = lp_tx.send(0.0);
                                    ctx.request_repaint();
                                    let _ = sleep(Duration::from_secs(10)).await;
                                    let _ = ot_tx.send("Flashing failed... Did you try unplugging it and plugging it back in?".to_string());
                                    return;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = ot_tx.send(e.to_string());
                            let _ = lp_tx.send(0.0);
                            ctx.request_repaint();
                            let _ = sleep(Duration::from_secs(10)).await;
                            let _ = ot_tx.send("Flashing failed... Did you try unplugging it and plugging it back in?".to_string());
                            return;
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
                    return;
                }
            }
        });
    }
}

impl App for FWUpdater {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
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
        }

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

                                ui.add_space(10.0);

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
                            .anchor(Align2::CENTER_CENTER, [0.0, 50.0])
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
                                        self.start_flash_op(
                                            *self.flasher.clone().lock().unwrap(),
                                            self.ot_tx.clone(),
                                            self.lp_tx.clone(),
                                            ctx.clone(),
                                        );
                                    }
                                });
                            });
                    });
                });
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
