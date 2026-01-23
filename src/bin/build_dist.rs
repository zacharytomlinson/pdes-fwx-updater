use std::{fs, path::Path};

fn main() -> anyhow::Result<()> {
    // -----------------------------
    // Configuration
    // -----------------------------
    let exe_source = "./target/x86_64-pc-windows-gnu/release/pdes-fwx-updater.exe";
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

    // -----------------------------
    // Optional: seed config for bootstrap installs
    // -----------------------------
    if let Ok(pkg_url) = std::env::var("PDES_PKG_URL") {
        if pkg_url.trim().is_empty() {
            println!("PDES_PKG_URL is set but empty; skipping seed config.");
        } else {
            let mut seed = serde_json::Map::new();
            seed.insert("bootstrap_pkg_url".to_string(), pkg_url.trim().into());

            if let Ok(v) = std::env::var("PDES_API_BASE_URL")
                && !v.trim().is_empty()
            {
                seed.insert("api_base_url".to_string(), v.trim().into());
            }
            if let Ok(v) = std::env::var("PDES_APP_SLUG")
                && !v.trim().is_empty()
            {
                seed.insert("app_slug".to_string(), v.trim().into());
            }
            if let Ok(v) = std::env::var("PDES_FIRMWARE_CHANNEL")
                && !v.trim().is_empty()
            {
                seed.insert(
                    "firmware".to_string(),
                    serde_json::json!({ "channel": v.trim() }),
                );
            }

            let seed_path = format!("{}/seed-config.json", dist_dir);
            fs::write(
                &seed_path,
                serde_json::to_vec_pretty(&serde_json::Value::Object(seed))?,
            )?;
            println!("Wrote seed config at {}", seed_path);
        }
    }

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
