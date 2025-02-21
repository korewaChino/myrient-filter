use myrient_filter::{FilterOptions, Rom, RomLister};
use std::path::{Path, PathBuf};

async fn download_file(
    url: &str,
    dest: PathBuf,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let response = client.get(url).send().await?;

    println!("url: {}", url);

    // Check if response is text based on content-type header
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if content_type.contains("text") {
        let text = response.text().await?;
        println!("Received text response:\n{}", text);
        Ok(None)
    } else {
        let bytes = response.bytes().await?;
        tokio::fs::write(&dest, &bytes).await?;
        Ok(Some(dest))
    }
}

async fn download_rom(rom: &Rom, download_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let dest = download_path.join(&rom.filename);
    println!("Downloading {} to {:?}", rom.filename, dest);

    if let Some(dest) = download_file(&rom.url, dest).await? {
        if dest.extension().unwrap_or_default() == "zip" {
            let dest_dir = dest.with_extension("");
            let status = std::process::Command::new("unzip")
                .arg("-o")
                .arg(&dest)
                .arg("-d")
                .arg(&dest_dir)
                .status()?;

            if !status.success() {
                eprintln!("Failed to unzip {:?}", dest);
                return Ok(());
            }

            tokio::fs::remove_file(&dest).await?;
            let filename_stripped = dest.file_stem().unwrap().to_str().unwrap();
            let resulting_folder = download_path.join(filename_stripped);
            if resulting_folder.exists() {
                println!("Moving files from {:?} to {:?}", dest_dir, resulting_folder);
                let mut entries = tokio::fs::read_dir(&resulting_folder).await?;
                while let Some(entry) = entries.next_entry().await? {
                    let entry_path = entry.path();
                    let entry_filename = entry_path.file_name().unwrap().to_str().unwrap();
                    let new_path = download_path.join(entry_filename);
                    tokio::fs::copy(&entry_path, &new_path).await?;
                }
                tokio::fs::remove_dir_all(&dest_dir).await?;
            }

            // println!("Resulting folder: {:?}", resulting_folder);
        }
    } else {
        eprintln!("Skipping {} - received text response", rom.filename);
    }

    Ok(())
}

// Download all SNES retail ROMs released in the USA
// excluding prototypes, betas, and other non-retail ROMs,
// and only get the latest release of each title (if multiple revisions exist)
// and save them to the "snes" directory

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = FilterOptions {
        region_limit: true,
        region: "USA".to_string(),
        smart_filters: true,
        exclude_patterns: vec![
            "Pirate".to_string(),
            "Beta".to_string(),
            "Proto".to_string(),
            "Enhancement Chip".to_string(),
            "Tech Demo".to_string(),
            "Competition Cart, Nintendo Power mail-order".to_string(),
            "Sample".to_string(),
            "Aftermarket".to_string(),
            "Demo".to_string(),
            "Unl".to_string(),
        ],
        latest_revision: true,
    };

    let lister = RomLister::new(options);

    // List base directories
    println!("Available base directories:");
    for dir in lister.list_directories(Some("No-Intro")).await? {
        println!("- {}", dir);
    }

    const SNES: &str = "Nintendo - Super Nintendo Entertainment System";
    const DOWNLOAD_PATH: &str = "snes/";
    let download_path = PathBuf::from(DOWNLOAD_PATH);

    println!("Listing ROMs for {}:", SNES);
    for rom in lister.list_roms(SNES, "No-Intro").await? {
        if let Err(e) = download_rom(&rom, &download_path).await {
            eprintln!("Failed to download {}: {}", rom.filename, e);
        }
    }

    Ok(())
}
