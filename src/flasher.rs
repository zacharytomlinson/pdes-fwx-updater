use std::collections::HashMap;
use std::fs;
use std::path::Path;
use probe_rs::flashing::{DownloadOptions, FlashProgress, ProgressEvent, FlashError};
use probe_rs::probe::{list::Lister, DebugProbeError, DebugProbeInfo, Probe};
use probe_rs::{Permissions, Session};
use serialport::{SerialPort, SerialPortType, UsbPortInfo};
use std::process::Stdio;
use std::thread::sleep;
use std::time::{Duration, Instant};
use anyhow::anyhow;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

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
        me.ports = Self::populate_ports();
        if me.selected_port.is_empty() && let Some((key, _value)) = me.ports.iter().next() {
            me.selected_port = key.to_string();
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

    fn populate_ports() -> HashMap<String, UsbPortInfo> {
        let mut nu = HashMap::<String, UsbPortInfo>::new();
        let ports = serialport::available_ports().expect("No ports found!");
        for p in ports {
            if let SerialPortType::UsbPort(info) = p.port_type {
               nu.insert(p.port_name.clone(), info);
            }
        }
        nu
    }

    pub fn connect_probe(&self) -> Result<Probe, DebugProbeError> {
        if self.flash_vector == FlashVector::Probe {
            self.probes[0].open()
        } else {
            Err(DebugProbeError::TargetNotFound)
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

    pub fn trigger_bootloader_via_usb(&self) -> anyhow::Result<()> {
        let _ = serialport::new(self.selected_port.clone(), 1200)
            .timeout(Duration::from_millis(100))
            .open();
        Ok(())
    }

    pub fn connect_to_bootloader(&mut self, timeout: u64) -> anyhow::Result<()> {
        let start = Instant::now();

        while start.elapsed() < Duration::from_secs(timeout) {
            let existing = self.ports.clone();
            let current = Self::populate_ports();
            for port in &current {
                if !existing.contains_key(port.0.as_str()) {
                    self.selected_port = port.0.to_string();
                    return Ok(());
                }
            }
            sleep(Duration::from_millis(250));
        }

        Err(anyhow!("Timed out waiting for bootloader port"))
    }

    pub async fn flash_via_usb(&self, path: String, mut on_line: impl FnMut(&str) + Send + 'static, ) -> anyhow::Result<()> {
        //"/home/arch/.arduino15/packages/arduino/tools/bossac/1.7.0-arduino3/bossac" -i -d --port=ttyACM0 -U true -i -e -w -v "/home/arch/.cache/arduino/sketches/A5A9B19BBB40C6A3656AB42EF73C1D25/Blink.ino.bin" -R
        let mut child = Command::new("bossac")
            .args([
                "-i",
                "-d",
                format!("--port={}", self.selected_port.trim_prefix("/dev/")).as_str(),
                "-U=true",
                "-e",
                "-w",
                "-v",
                "--offset=0x2000",
                path.as_str(),
                "-R",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().ok_or_else(|| anyhow!("No stdout"))?;
        let stderr = child.stderr.take().ok_or_else(|| anyhow!("No stderr"))?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let mut stderr_reader = BufReader::new(stderr).lines();

        // Read stdout + stderr concurrently
        loop {
            tokio::select! {
            line = stdout_reader.next_line() => {
                match line? {
                    Some(l) => on_line(l.as_str()),
                    None => break,
                }
            }
            line = stderr_reader.next_line() => {
                match line? {
                    Some(l) => on_line(format!("ERR: {}", l).as_str()),
                    None => break,
                }
            }
        }
        }

        let status = child.wait().await?;
        if !status.success() {
            return Err(anyhow!(status.to_string()));
        }

        Ok(())
    }
}
