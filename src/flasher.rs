use anyhow::anyhow;
use probe_rs::flashing::{DownloadOptions, FlashError, FlashProgress, ProgressEvent};
use probe_rs::probe::{DebugProbeError, DebugProbeInfo, Probe, list::Lister};
use probe_rs::{Permissions, Session};
use serialport::{SerialPortType, UsbPortInfo};
use std::collections::HashMap;
use std::env;
use std::fs;
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
    pub(crate) ports: HashMap<String, UsbPortInfo>,
    pub(crate) selected_port: String,
    pub(crate) flash_vector: FlashVector,
}

impl Default for Flasher {
    fn default() -> Self {
        let mut me: Flasher = Flasher {
            chip: ChipModel::ATSAMD21G18A,
            probes: Vec::new(),
            ports: HashMap::new(),
            selected_port: String::new(),
            flash_vector: FlashVector::Probe,
        };
        me.ports = Self::populate_ports();
        if me.selected_port.is_empty()
            && let Some((key, _value)) = me.ports.iter().next()
        {
            me.selected_port = key.to_string();
        }

        let lister: Lister = Lister::new();
        let probe_list: Vec<DebugProbeInfo> = lister.list_all();
        if probe_list.is_empty() {
            me.flash_vector = FlashVector::Usb;
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
        match self.chip {
            ChipModel::ATSAMD21G18A => probe.attach("ATSAMD21G18A", Permissions::default()),
            ChipModel::STM32F103C8 => probe.attach("STM32F2103C8", Permissions::default()),
        }
    }

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

    pub async fn flash_via_usb(
        &self,
        bin_path: String,
        mut on_line: impl FnMut(&str) + Send + 'static,
    ) -> anyhow::Result<()> {
        let bossac_path = Self::bossac_path();

        //"/home/arch/.arduino15/packages/arduino/tools/bossac/1.7.0-arduino3/bossac" -i -d --port=ttyACM0 -U true -i -e -w -v "/home/arch/.cache/arduino/sketches/A5A9B19BBB40C6A3656AB42EF73C1D25/Blink.ino.bin" -R
        #[cfg(target_os = "linux")]
        let dev_path: String = format!("--port={}", self.selected_port.trim_prefix("/dev/"));
        #[cfg(target_os = "windows")]
        let dev_path: String = format!("--port={}", self.selected_port);

        let mut child = Command::new(bossac_path)
            .args([
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

    /// Locate bundled bossac before falling back to PATH (dev installs may still rely on PATH).
    fn bossac_path() -> PathBuf {
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

        if let Ok(exe_path) = env::current_exe() {
            if let Some(dir) = exe_path.parent() {
                let bundled = dir.join("tools").join(bossac_name);
                if bundled.exists() {
                    return bundled;
                }

                // Debian package installs tools under /opt
                let opt_bundled = Path::new("/opt/pdes-win-fw-updater/tools").join(bossac_name);
                if opt_bundled.exists() {
                    return opt_bundled;
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
        }

        println!("Bundled bossac not found; falling back to PATH");
        PathBuf::from("bossac")
    }
}

#[cfg(test)]
mod tests {
    use super::Flasher;
    use std::env;
    use std::fs;
    use std::path::PathBuf;

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
        let _ = fs::remove_file(PathBuf::from(forced));
    }
}
