use std::fs;
use std::path::Path;
use probe_rs::flashing::{Format, ElfOptions, FileDownloadError, download_file_with_options, DownloadOptions, FlashProgress, ProgressEvent, FlashError};
use probe_rs::probe::{list::Lister, DebugProbeError, DebugProbeInfo, Probe};
use probe_rs::{Permissions, Session, SessionConfig};

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum ChipModel {
    ATSAMD21G18A,
    STM32F103C8,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Flasher {
    pub(crate) chip: ChipModel,
}

impl Default for Flasher {
    fn default() -> Self {
        Self {
            chip: ChipModel::ATSAMD21G18A
        }
    }
}

impl Flasher {

    pub fn connect(&self) -> Result<Probe, DebugProbeError> {
        let lister: Lister = Lister::new();

        let probe_list: Vec<DebugProbeInfo> = lister.list_all();
        for probe_info in probe_list.clone() {
            println!("{:?}", probe_info.identifier);
        }
        if probe_list.is_empty() {
            return Err(DebugProbeError::TargetNotFound)
        }
        probe_list[0].open()
    }

    pub fn attach(&self, probe: Probe) -> Result<Session, probe_rs::Error> {
        // Attach to a chip.
        let si: Result<Session, probe_rs::Error>;
        match self.chip {
            ChipModel::ATSAMD21G18A => si = probe.attach("ATSAMD21G18A", Permissions::default()),
            ChipModel::STM32F103C8 => si = probe.attach("STM32F2103C8", Permissions::default())
        }
        si
    }

    pub fn flash(&self, mut session: Session, path: String, handler: impl FnMut(ProgressEvent)) -> Result<(), FlashError> {
        let file_path = Path::new(path.as_str());
        let data = fs::read(file_path);
        if data.is_err() {
            println!("Unable to read flash data");
            return Err(FlashError::PageWrite { page_address: 0, source: Box::new(data.unwrap_err()) });
        }

        let mut loader = session.target().flash_loader();
        loader.add_data(0x2000, &data.unwrap())?;

        let mut opts = DownloadOptions::new();
        opts.do_chip_erase = false;
        opts.progress = FlashProgress::new(handler);
        loader.commit(&mut session, opts)
    }
}
