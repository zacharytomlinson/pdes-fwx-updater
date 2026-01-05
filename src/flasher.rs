use std::collections::HashMap;
use std::fs;
use std::path::Path;
use probe_rs::flashing::{DownloadOptions, FlashProgress, ProgressEvent, FlashError};
use probe_rs::probe::{list::Lister, DebugProbeError, DebugProbeInfo, Probe};
use probe_rs::{Permissions, Session};
use serialport::{ErrorKind, SerialPortType, UsbPortInfo};
use cxx::{let_cxx_string, UniquePtr};
use bossa::lib;
use std::ffi::CString;
use std::os::raw::c_char;

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
                me.ports.insert(p.port_name.clone(), info);
                if me.selected_port.is_empty() {
                    me.selected_port = p.port_name.clone();
                }
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
        let_cxx_string!(port = self.selected_port.clone());

        // Attempt to connect to the usb port
        if !samba
            .pin_mut()
            .connect(port_factory.pin_mut().create_port(&port, true), 115200)
        {
            println!("Failed to connect to port: {}", port);
            Err(ErrorKind::NoDevice)
        } else {
            println!("Successfully connected to port: {}", port);
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

    pub fn attach_usb(&self, mut samba: UniquePtr<lib::Samba>) -> UniquePtr<lib::Flasher> {
        let mut device = lib::new_device(samba.pin_mut());
        device.pin_mut().create();
        let mut observer = lib::new_bossa_observer();
        let mut flasher = lib::new_flasher(samba.pin_mut(), device.pin_mut(), observer.pin_mut());
        let mut flasher_info = lib::new_flasher_info();
        flasher.pin_mut().info(flasher_info.pin_mut());
        flasher_info.pin_mut().print();
        flasher
    }

    pub fn flash_via_probe(&self, mut session: Session, path: String, handler: impl FnMut(ProgressEvent)) -> Result<(), FlashError> {
        let file_path = Path::new(path.as_str());
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
    }

    pub unsafe fn flash_via_usb(&self, mut samba_flash: UniquePtr<lib::Flasher>, path: String) -> u32 {
        let c_str: CString = CString::new(path).expect("CString::new failed");
        let ptr: *const c_char = c_str.as_ptr();
        let mut pe: u32 = 0u32;
        let mut te: u32 = 0u32;
        let mut flasher_info = lib::new_flasher_info();
        let mut info = lib::FlasherInfoRs::default();
        lib::flasherinfo2flasherinfors(&flasher_info.pin_mut(), &mut info);
        println!("{:#?}", info);
        println!("Unique Id: {}", info.unique_id());
        samba_flash.pin_mut().reset();
        unsafe {
            samba_flash.pin_mut().write(ptr, 0x2000);
            samba_flash.pin_mut().verify(ptr, &mut pe, &mut te, 0x2000);
            te
        }
    }
}
