use std::collections::HashMap;
use std::fs;
use std::path::Path;
use probe_rs::flashing::{DownloadOptions, FlashProgress, ProgressEvent, FlashError};
use probe_rs::probe::{list::Lister, DebugProbeError, DebugProbeInfo, Probe};
use probe_rs::{Permissions, Session};
use serialport::{ErrorKind, SerialPortType, UsbPortInfo};
use cxx::{let_cxx_string, UniquePtr};
use bossa::lib;

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum ChipModel {
    ATSAMD21G18A,
    STM32F103C8,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum FlashVector {
    Probe,
    USB,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Flasher {
    pub(crate) chip: ChipModel,
    pub(crate) probes: Vec<DebugProbeInfo>,
    pub(crate) ports: HashMap<String, UsbPortInfo>,
    pub(crate) selected_port: String,
    pub(crate) flash_vector: FlashVector
}

impl Default for Flasher {
    fn default() -> Self {
        let mut me: Flasher = Flasher {
            chip: ChipModel::ATSAMD21G18A,
            probes: Vec::new(),
            ports: HashMap::new(),
            selected_port: String::new(),
            flash_vector: FlashVector::Probe
        };
        let ports = serialport::available_ports().expect("No ports found!");
        for p in ports {
            if let SerialPortType::UsbPort(info) = p.port_type {
                me.ports.insert(p.port_name, info);
            }
        }
        let lister: Lister = Lister::new();
        let probe_list: Vec<DebugProbeInfo> = lister.list_all();
        if probe_list.is_empty() {
            me.flash_vector = FlashVector::USB;
            me
        } else {
            for probe_info in probe_list.clone() {
                println!("{:?}", probe_info.identifier);
                me.probes.push(probe_info);
            }
            me.flash_vector = FlashVector::Probe;
            me
        }
    }
}

impl Flasher {

    pub fn connect_probe(&self) -> Result<Probe, DebugProbeError> {
        if self.flash_vector == FlashVector::Probe {
            self.probes[0].open()
        } else {
            Err(DebugProbeError::TargetNotFound)
        }
    }

    pub fn connect_usb(&self) -> Result<UniquePtr<lib::Samba>, ErrorKind> {
        let mut samba = lib::new_samba();
        let mut port_factory = lib::new_port_factory();

        // Use default port (e.g. /dev/ttyACM0 on linux)
        let_cxx_string!(port = port_factory.pin_mut().default_name().to_string());

        // Attempt to connect to the usb port
        if !samba
            .pin_mut()
            .connect(port_factory.pin_mut().create_port(&port, true), 115200)
        {
            println!("Failed to connect to port: {}", port);
            Err(ErrorKind::NoDevice)
        } else {
            Ok(samba)
        }
    }

    pub fn attach_probe(&self, probe: Probe) -> Result<Session, probe_rs::Error> {
        // Attach to a chip.
        let si: Result<Session, probe_rs::Error>;
        match self.chip {
            ChipModel::ATSAMD21G18A => si = probe.attach("ATSAMD21G18A", Permissions::default()),
            ChipModel::STM32F103C8 => si = probe.attach("STM32F2103C8", Permissions::default())
        }
        si
    }

    pub fn attach_usb(&self, samba: lib::Samba) -> Result<(), i32> {

    }

    pub fn flash(&self, mut session: Session, path: String, handler: impl FnMut(ProgressEvent)) -> Result<(), FlashError> {
        let file_path = Path::new(path.as_str());
        if self.flash_vector == FlashVector::Probe {
            let data = fs::read(file_path);
            if data.is_err() {
                println!("Unable to read flash data");
                return Err(FlashError::PageWrite { page_address: 0, source: Box::new(data.unwrap_err()) });
            }

            let mut loader = session.target().flash_loader();
            loader.add_data(0x2000, &data.unwrap())?;

            let mut opts = DownloadOptions::new();
            opts.do_chip_erase = true;
            opts.verify = true;
            opts.progress = FlashProgress::new(handler);
            loader.commit(&mut session, opts)
        } else {

        }
    }
}
