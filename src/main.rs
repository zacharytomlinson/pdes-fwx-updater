#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod flasher;

use std::ops::Deref;
use flasher::{ Flasher };
use eframe::{egui, App, CreationContext, NativeOptions};
use egui::{Align2, Area, CentralPanel, Context, Id};
use crate::flasher::ChipModel;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Sender, Receiver};
use std::time::Duration;
use egui_file_dialog::FileDialog;
use tokio::runtime::Runtime;
use tokio::time::sleep;

#[derive(Default, Debug, Copy, Clone)]
pub enum ViewState {
  #[default]
  Splash,
  Main,
}

#[derive(Default, Debug)]
pub enum FlashStatus {
  #[default]
  Init,
  ProbeConnecting(String),
  ProbeConnectError(String),
  TargetAttaching(String),
  TargetAttachError(String),
  Flashing(String),
  FlashError(String),
  Complete(String),
}

//the main application struct, holding its current screen
struct FWUpdater {
  state: Arc<Mutex<ViewState>>,
  load_progress: Arc<Mutex<f32>>,
  flasher: Arc<Mutex<Flasher>>,
  elf_path: Arc<Mutex<Option<PathBuf>>>,
  fd: Arc<Mutex<FileDialog>>,
  fd_text: String,
  show_output: bool,
  output_text: Arc<Mutex<String>>,
  flash_stage: u8,

  lp_tx: Sender<f32>,
  lp_rx: Receiver<f32>,

  stat_tx: Sender<ViewState>,
  stat_rx: Receiver<ViewState>,

  status_tx: Sender<FlashStatus>,
  status_rx: Receiver<FlashStatus>,
}

impl Default for FWUpdater {
  fn default() -> Self {
    let (lp_tx, lp_rx) = channel();
    let (stat_tx, stat_rx) = channel();
    let (status_tx, status_rx) = channel();
    Self {
      state: Arc::new(Mutex::new(ViewState::Splash)),
      load_progress: Arc::new(Mutex::new(0.0)),
      flasher: Default::default(),
      elf_path: Arc::new(Mutex::new(None)),
      fd: Default::default(),
      fd_text: "Select Binary to Flash".to_string(),
      show_output: false,
      output_text: Arc::new(Mutex::new("Initializing Target...".to_string())),
      flash_stage: 0,

      lp_rx,
      lp_tx,
      stat_tx,
      stat_rx,
      status_tx,
      status_rx
    }
  }
}

impl FWUpdater {
  fn new(_cc: &CreationContext<'_>) -> Self {
    Default::default()
  }

  // Simulate loading or perform actual loading tasks here
  fn update_progress_bar(&mut self, stat_tx: Sender<ViewState>, lp_tx: Sender<f32>, ctx: Context) {
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

  fn start_flash_op(&mut self, flash: Flasher, stage_tx: Sender<FlashStatus>, lp_tx: Sender<f32>, ctx: Context) {
    let prog = *self.load_progress.clone().lock().unwrap();
    let path = self.elf_path.lock().unwrap().as_ref().unwrap().display().to_string();
    tokio::spawn(async move {
      let _ = stage_tx.send(FlashStatus::ProbeConnecting("Connecting to Probe...".to_string()));
      ctx.request_repaint();
      match flash.connect() {
        Ok(probe) => {
          let _ = lp_tx.send(prog + 40.0);
          let _ = stage_tx.send(FlashStatus::TargetAttaching("Attaching Probe to Target...".to_string()));
          ctx.request_repaint();
          match flash.attach(probe) {
            Ok(session) => {
              let _ = lp_tx.send(prog + 5.0);
              let _ = stage_tx.send(FlashStatus::Flashing("Flashing Binary to Target".to_string()));
              ctx.request_repaint();
              match flash.flash(session, path,|evt| {
                println!("{:?}", evt);
                let _ = stage_tx.send(FlashStatus::Flashing(format!("{:#?}", evt)));
                ctx.request_repaint();
              }) {
                Ok(_) => {
                  let _ = lp_tx.send(prog + 5.0);
                  let _ = stage_tx.send(FlashStatus::Complete("Flashing Successful!".to_string()));
                  ctx.request_repaint();
                },
                Err(e) => {
                  let _ = stage_tx.send(FlashStatus::FlashError(e.to_string()));
                  let _ = lp_tx.send(0.0);
                  ctx.request_repaint();
                  let _ = sleep(Duration::from_secs(10)).await;
                  let _ = stage_tx.send(FlashStatus::Complete("Flashing failed... Did you try unplugging it and plugging it back in?".to_string()));
                  return;
                }
              }
            },
            Err(e) => {
              let _ = stage_tx.send(FlashStatus::TargetAttachError(e.to_string()));
              let _ = lp_tx.send(0.0);
              ctx.request_repaint();
              let _ = sleep(Duration::from_secs(10)).await;
              let _ = stage_tx.send(FlashStatus::Complete("Flashing failed... Did you try unplugging it and plugging it back in?".to_string()));
              return;
            }
          }
        },
        Err(e) => {
          let _ = stage_tx.send(FlashStatus::ProbeConnectError(e.to_string()));
          let _ = lp_tx.send(0.0);
          ctx.request_repaint();
          let _ = sleep(Duration::from_secs(10)).await;
          let _ = stage_tx.send(FlashStatus::Complete("Flashing failed... Did you try unplugging it and plugging it back in?".to_string()));
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
    if let Ok(response) = self.status_rx.try_recv() {
      match response {
        FlashStatus::ProbeConnecting(msg) => {
          if self.flash_stage < 1 {
            *self.output_text.clone().lock().unwrap() = msg;
            self.flash_stage = 1;
          }
        },
        FlashStatus::ProbeConnectError(msg) => {
          if self.flash_stage < 2 {
            *self.output_text.clone().lock().unwrap() = msg;
            self.flash_stage = 2;
          }
        },
        FlashStatus::TargetAttaching(msg) => {
          if self.flash_stage < 3 {
            *self.output_text.clone().lock().unwrap() = msg;
            self.flash_stage = 3;
          }
        },
        FlashStatus::TargetAttachError(msg) => {
          if self.flash_stage < 4 {
            *self.output_text.clone().lock().unwrap() = msg;
            self.flash_stage = 4;
          }
        },
        FlashStatus::Flashing(msg) => {
          if self.flash_stage < 5 {
            *self.output_text.clone().lock().unwrap() = msg;
            self.flash_stage = 5;
          }
        },
        FlashStatus::FlashError(msg) => {
          if self.flash_stage < 6 {
            *self.output_text.clone().lock().unwrap() = msg;
            self.flash_stage = 6;
          }
        },
        FlashStatus::Complete(msg) => {
          *self.output_text.clone().lock().unwrap() = "Flashing Completed!".to_string();
          *self.load_progress.clone().lock().unwrap() += 50.0;
          self.flash_stage = 7;
        }
        _ => {
          *self.output_text.clone().lock().unwrap() = "Initializing...".to_string();
          self.flash_stage = 0;
        }
      }
    }

    match *self.state.clone().lock().unwrap() {
      ViewState::Splash => {
        CentralPanel::default().show(ctx, |ui| {
          ui.vertical_centered(|ui| {
            ui.add_space(100.0);
            ui.add(
              egui::Image::new(egui::include_image!("../assets/pdes_tx_logo.png"))
                .corner_radius(5),
            );
            ui.add_space(20.0);
            ui.add(egui::ProgressBar::new(*self.load_progress.lock().unwrap()).show_percentage());
            ui.add_space(20.0);
          });

          self.update_progress_bar(self.stat_tx.clone(), self.lp_tx.clone(), ctx.clone());
        });
      }
      ViewState::Main => {
        CentralPanel::default().show(ctx, |ui| {
          ui.vertical_centered(|ui| {
            ui.add(
              egui::Image::new(egui::include_image!("../assets/pdes_tx_logo.png"))
                .max_height(45.0)
            );
            ui.add_space(50.0);

            egui::ComboBox::from_label("Chip:")
              .selected_text(format!("{:?}",  self.flasher.lock().unwrap().chip.clone()))
              .width(ui.available_width()/3.0)
              .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.flasher.lock().unwrap().chip, ChipModel::ATSAMD21G18A, "ATSAMD21");
                ui.selectable_value(&mut self.flasher.lock().unwrap().chip, ChipModel::STM32F103C8, "STM32F2");
              });
            ui.add_space(50.0);

            if ui.add(egui::TextEdit::singleline(&mut self.fd_text).clip_text(false)).clicked() {
              self.fd.lock().unwrap().pick_file();
            };

            if let Some(path) = self.fd.lock().unwrap().take_picked() {
              self.elf_path = Arc::new(Mutex::new(Some(path.to_path_buf())));
              self.fd_text = path.to_str().unwrap().to_string();
            }

            ui.add_space(50.0);
            Area::new(Id::from("bottom_center_area"))
              .anchor(Align2::CENTER_BOTTOM, [0.0, -50.0])
              .show(ui.ctx(), |ui| {
                ui.horizontal_centered(|ui| {
                  if ui.button("FLASH").clicked() {
                    self.show_output = !self.show_output;
                  }
                  if self.show_output {
                    ui.add(egui::Label::new(self.output_text.clone().lock().unwrap().deref().clone()));
                    ui.add(egui::ProgressBar::new(*self.load_progress.lock().unwrap()).show_percentage());

                    if self.flash_stage == 0 || self.flash_stage == 7 {
                      self.start_flash_op(*self.flasher.clone().lock().unwrap(), self.status_tx.clone(), self.lp_tx.clone(), ctx.clone());
                    }
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
