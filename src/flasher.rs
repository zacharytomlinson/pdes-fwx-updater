//! PDES Firmware Updater flashing utilities.
//! Author: Zachary Tomlinson <ztomlinson7@gmail.com>
//! Version: v1.0.0
//! Date: 2025-02-12
//! Handles probe-rs flashing and USB bossac invocation with bundled-tool lookup
//! for both Windows and Linux distributions.
use anyhow::{Context, anyhow};
use probe_rs::flashing::{DownloadOptions, FlashError, FlashProgress, ProgressEvent};
use probe_rs::probe::{DebugProbeError, DebugProbeInfo, Probe, list::Lister};
use probe_rs::{Permissions, Session};
use serialport::{ClearBuffer, SerialPortType, UsbPortInfo};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread::sleep;
use std::time::{Duration, Instant};
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
    Usb,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Flasher {
    pub(crate) chip: ChipModel,
    pub(crate) probes: Vec<DebugProbeInfo>,
    pub(crate) selected_probe_index: usize,
    pub(crate) ports: HashMap<String, UsbPortInfo>,
    pub(crate) selected_port: String,
    pub(crate) flash_vector: FlashVector,
}

impl Default for Flasher {
    fn default() -> Self {
        let mut me: Flasher = Flasher {
            chip: ChipModel::ATSAMD21G18A,
            probes: Vec::new(),
            selected_probe_index: 0,
            ports: HashMap::new(),
            selected_port: String::new(),
            flash_vector: FlashVector::Usb,
        };
        me.refresh_devices();
        me
    }
}

impl Flasher {
    /// Discover available USB serial ports and map them by port name.
    fn populate_ports() -> HashMap<String, UsbPortInfo> {
        let mut nu = HashMap::<String, UsbPortInfo>::new();
        match serialport::available_ports() {
            Ok(ports) => {
                for port in ports {
                    if let SerialPortType::UsbPort(info) = port.port_type {
                        nu.insert(port.port_name.clone(), info);
                    }
                }
            }
            Err(e) => {
                println!("Unable to enumerate serial ports: {e}");
            }
        }
        nu
    }

    pub fn refresh_devices(&mut self) {
        self.ports = Self::populate_ports();
        if self.selected_port.is_empty() || !self.ports.contains_key(&self.selected_port) {
            self.selected_port = self.ports.keys().next().cloned().unwrap_or_default();
        }

        let lister: Lister = Lister::new();
        self.probes = lister.list_all();
        if self.selected_probe_index >= self.probes.len() {
            self.selected_probe_index = 0;
        }

        let has_ports = !self.ports.is_empty();
        let has_probes = !self.probes.is_empty();
        match self.flash_vector {
            FlashVector::Usb if !has_ports && has_probes => self.flash_vector = FlashVector::Probe,
            FlashVector::Probe if !has_probes && has_ports => self.flash_vector = FlashVector::Usb,
            FlashVector::Probe if !has_probes && !has_ports => self.flash_vector = FlashVector::Usb,
            _ => {}
        }
    }

    /// Open the first detected debug probe when flashing via probe.
    pub fn connect_probe(&self) -> Result<Probe, DebugProbeError> {
        if self.flash_vector == FlashVector::Probe {
            self.probes
                .get(self.selected_probe_index)
                .ok_or(DebugProbeError::TargetNotFound)?
                .open()
        } else {
            Err(DebugProbeError::TargetNotFound)
        }
    }

    /// Attach a probe-rs session to the selected chip model.
    pub fn attach_probe(&self, probe: Probe) -> Result<Session, probe_rs::Error> {
        // Attach to a chip.
        match self.chip {
            ChipModel::ATSAMD21G18A => probe.attach("ATSAMD21G18A", Permissions::default()),
            ChipModel::STM32F103C8 => probe.attach("STM32F2103C8", Permissions::default()),
        }
    }

    /// Flash the binary via probe-rs with progress reporting.
    pub fn flash_via_probe(
        &self,
        mut session: Session,
        path: String,
        handler: impl FnMut(ProgressEvent),
    ) -> Result<(), FlashError> {
        let file_path = Path::new(path.as_str());
        let data = fs::read(file_path).map_err(|e| {
            println!("Unable to read flash data");
            FlashError::PageWrite {
                page_address: 0,
                source: Box::new(e),
            }
        })?;

        let mut loader = session.target().flash_loader();
        loader.add_data(0x2000, &data)?;

        let mut opts = DownloadOptions::new();
        opts.do_chip_erase = true;
        opts.verify = true;
        opts.progress = FlashProgress::new(handler);
        loader.commit(&mut session, opts)
    }

    /// Toggle a USB device into bootloader mode by opening at 1200 baud.
    pub fn trigger_bootloader_via_usb(&self) -> anyhow::Result<()> {
        let _ = serialport::new(self.selected_port.clone(), 1200)
            .timeout(Duration::from_millis(100))
            .open();
        Ok(())
    }

    /// Wait for the bootloader device to enumerate within the timeout.
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

    pub fn query_bootloader_hw_rev(&self) -> anyhow::Result<Option<u32>> {
        let mut port = serialport::new(self.selected_port.clone(), 115_200)
            .timeout(Duration::from_millis(300))
            .open()
            .with_context(|| {
                format!(
                    "Unable to open bootloader port {} for hardware revision query",
                    self.selected_port
                )
            })?;

        let _ = port.clear(ClearBuffer::All);
        port.write_all(b"I#")
            .context("Unable to send hardware revision query")?;
        port.flush()
            .context("Unable to flush hardware revision query")?;

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut response = Vec::with_capacity(16);
        let mut buf = [0u8; 64];

        while Instant::now() < deadline {
            match port.read(&mut buf) {
                Ok(0) => {}
                Ok(n) => {
                    response.extend_from_slice(&buf[..n]);
                    if response.contains(&b'#') {
                        break;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(e) => return Err(e).context("Unable to read hardware revision response"),
            }
        }

        parse_bootloader_hw_rev_response(&response)
    }

    /// Flash via USB using bossac, streaming output to the callback.
    pub async fn flash_via_usb(
        &self,
        bin_path: String,
        mut on_line: impl FnMut(&str) + Send + 'static,
    ) -> anyhow::Result<()> {
        let bossac_path = Self::bossac_path();

        //"/home/arch/.arduino15/packages/arduino/tools/bossac/1.7.0-arduino3/bossac" -i -d --port=ttyACM0 -U true -i -e -w -v "/home/arch/.cache/arduino/sketches/A5A9B19BBB40C6A3656AB42EF73C1D25/Blink.ino.bin" -R
        #[cfg(target_os = "linux")]
        let dev_path: String = format!(
            "--port={}",
            self.selected_port
                .strip_prefix("/dev/")
                .unwrap_or(&self.selected_port)
        );
        #[cfg(target_os = "windows")]
        let dev_path: String = format!("--port={}", self.selected_port);

        let mut cmd = Command::new(bossac_path);
        cmd.args([
            dev_path.as_str(),
            "-i",
            "-d",
            "-e",
            "-w",
            "-v",
            "--offset=0x2000",
            bin_path.as_str(),
            "-R",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        #[cfg(target_os = "windows")]
        {
            // CREATE_NO_WINDOW to prevent flashing a console window during bossac execution.
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn()?;

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

    /// Locate bundled bossac before falling back to PATH (dev installs may still rely on PATH).
    pub fn bossac_path() -> PathBuf {
        let bossac_name = if cfg!(windows) {
            "bossac.exe"
        } else {
            "bossac"
        };

        if let Ok(forced) = env::var("PDES_BOSSAC_PATH") {
            let forced_path = PathBuf::from(forced);
            if forced_path.exists() {
                return forced_path;
            }
        }

        if let Ok(exe_path) = env::current_exe()
            && let Some(dir) = exe_path.parent()
        {
            let bundled = dir.join("tools").join(bossac_name);
            if bundled.exists() {
                return bundled;
            }

            // Debian package installs tools under /opt
            for root in [
                "/opt/pdes-fwx-updater/tools",
                "/opt/pdes-win-fw-updater/tools",
            ] {
                let opt_bundled = Path::new(root).join(bossac_name);
                if opt_bundled.exists() {
                    return opt_bundled;
                }
            }

            // Dev tree: repo_root/tools/{windows|linux}/bossac*
            if let Some(repo_root) = dir.parent().and_then(|p| p.parent()) {
                let repo_bundled = repo_root
                    .join("tools")
                    .join(if cfg!(windows) { "windows" } else { "linux" })
                    .join(bossac_name);
                if repo_bundled.exists() {
                    return repo_bundled;
                }
            }
        }

        println!("Bundled bossac not found; falling back to PATH");
        PathBuf::from("bossac")
    }
}

fn parse_bootloader_hw_rev_response(response: &[u8]) -> anyhow::Result<Option<u32>> {
    let text = String::from_utf8_lossy(response);
    let start = text
        .find('I')
        .ok_or_else(|| anyhow!("Hardware revision response missing I prefix"))?;
    let text = &text[start + 1..];
    let end = text
        .find('#')
        .ok_or_else(|| anyhow!("Hardware revision response missing terminator"))?;
    let hex = text[..end].trim();

    if hex.len() != 8 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("Unexpected hardware revision payload: {hex}");
    }

    let value = u32::from_str_radix(hex, 16)
        .with_context(|| format!("Invalid hardware revision payload: {hex}"))?;
    if value == 0xFFFF_FFFF {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::{Flasher, parse_bootloader_hw_rev_response};
    use std::env;
    use std::fs;

    #[test]
    fn bossac_path_respects_env_override() {
        let tmp_dir = env::temp_dir();
        let forced = tmp_dir.join("bossac-test-bin");
        fs::write(&forced, b"test").expect("unable to write temp bossac");

        let original = env::var("PDES_BOSSAC_PATH").ok();
        unsafe {
            env::set_var("PDES_BOSSAC_PATH", &forced);
        }

        let resolved = Flasher::bossac_path();

        // Restore env
        if let Some(val) = original {
            unsafe {
                env::set_var("PDES_BOSSAC_PATH", val);
            }
        } else {
            unsafe {
                env::remove_var("PDES_BOSSAC_PATH");
            }
        }

        assert_eq!(resolved, forced);

        // cleanup temp file
        let _ = fs::remove_file(forced);
    }

    #[test]
    fn parses_bootloader_hw_rev_response() {
        assert_eq!(
            parse_bootloader_hw_rev_response(b"I0000002A#\r\n").unwrap(),
            Some(0x2A)
        );
        assert_eq!(
            parse_bootloader_hw_rev_response(b"\r\nIFFFFFFFF#\r\n").unwrap(),
            None
        );
    }

    #[test]
    fn rejects_malformed_bootloader_hw_rev_response() {
        assert!(parse_bootloader_hw_rev_response(b"I123#").is_err());
        assert!(parse_bootloader_hw_rev_response(b"garbage").is_err());
    }
}
