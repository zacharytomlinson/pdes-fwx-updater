use std::{fs, path::Path};

fn main() -> anyhow::Result<()> {
    // -----------------------------
    // Configuration
    // -----------------------------
    let exe_source = "./target/x86_64-pc-windows-gnu/release/pdes-win-fw-updater.exe";
    let dist_dir = "./dist/PDES";

    let assets_dir = "./assets";

    // Now copy from tools/windows only for Windows distribution
    let tools_windows_dir_source = "./tools/windows";

    // -----------------------------
    // Create distribution folder
    // -----------------------------
    if Path::new(dist_dir).exists() {
        fs::remove_dir_all(dist_dir)?;
    }
    fs::create_dir_all(dist_dir)?;

    // -----------------------------
    // Copy main Windows executable
    // -----------------------------
    let exe_dest = format!("{}/pdes.exe", dist_dir);
    fs::copy(exe_source, exe_dest)?;

    // -----------------------------
    // Copy assets
    // -----------------------------
    let assets_dest = format!("{}/assets", dist_dir);
    fs::create_dir_all(&assets_dest)?;
    copy_dir_recursively(assets_dir, &assets_dest)?;

    // -----------------------------
    // Copy Windows tools from tools/windows
    // -----------------------------
    let tools_dest = format!("{}/tools", dist_dir);
    fs::create_dir_all(&tools_dest)?;
    copy_dir_recursively(tools_windows_dir_source, &tools_dest)?;

    println!("Distribution folder prepared at {}", dist_dir);
    Ok(())
}

// -----------------------------
// Helper function: recursively copy directories
// -----------------------------
fn copy_dir_recursively(src: &str, dst: &str) -> anyhow::Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = Path::new(dst).join(entry.file_name());

        if ft.is_dir() {
            fs::create_dir_all(&dst_path)?;
            copy_dir_recursively(src_path.to_str().unwrap(), dst_path.to_str().unwrap())?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}
