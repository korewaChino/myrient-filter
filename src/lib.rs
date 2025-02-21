use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use url::Url;
pub const NO_INTRO_DIR: &str = "No-Intro";
pub const BASE_URL: &str = "https://myrient.erista.me/files/";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterOptions {
    pub region_limit: bool,
    pub region: String,
    pub smart_filters: bool,
    pub exclude_patterns: Vec<String>,
    pub latest_revision: bool,
}

#[derive(Debug, Clone)]
pub struct Rom {
    pub filename: String,
    pub url: String,
}

#[derive(Debug)]
pub struct RomLister {
    client: Client,
    options: FilterOptions,
}

impl RomLister {
    pub fn new(options: FilterOptions) -> Self {
        Self {
            client: Client::new(),
            options,
        }
    }

    /// List directories at the given path. If no path is provided, lists directories at the base URL
    pub async fn list_directories(
        &self,
        subdir: Option<&str>,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let url = match subdir {
            Some(path) => format!("{}{}/", BASE_URL, path),
            None => BASE_URL.to_string(),
        };
        println!("Fetching directories from: {}", url);

        let response = self.client.get(&url).send().await?.text().await?;
        let document = Html::parse_document(&response);
        let selector = Selector::parse("tbody > tr > td.link > a").unwrap();

        let dirs: Vec<String> = document
            .select(&selector)
            .skip(1) // Skip parent directory link
            .filter(|link| {
                let href = link.value().attr("href").unwrap_or("");
                href.ends_with('/') // Only include directory entries
            })
            .map(|link| {
                let href = link.value().attr("href").unwrap();
                let trimmed = href.trim_end_matches('/');
                urlencoding::decode(trimmed)
                    .expect("Invalid UTF-8")
                    .into_owned()
            })
            .collect();

        Ok(dirs)
    }

    pub async fn list_rom_urls(
        &self,
        system: &str,
        subdir: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let encoded_system = system.replace(" ", "%20");
        let url = format!("{}{}/{}/", BASE_URL, subdir, encoded_system);
        println!("Fetching ROMs from: {}", url);

        let response = self.client.get(&url).send().await?.text().await?;
        let document = Html::parse_document(&response);
        let selector = Selector::parse("tbody > tr > td.link > a").unwrap();

        let urls: Vec<String> = document
            .select(&selector)
            .skip(1)
            .filter(|link| self.is_valid_file(link.value().attr("href").unwrap_or("")))
            .map(|link| {
                let href = link.value().attr("href").unwrap();
                if !href.starts_with("http") {
                    // Construct full URL preserving the system path
                    format!("{}{}/{}/{}", BASE_URL, subdir, encoded_system, href)
                } else {
                    href.to_string()
                }
            })
            .collect();

        Ok(urls)
    }

    pub async fn list_roms(
        &self,
        system: &str,
        subdir: &str,
    ) -> Result<Vec<Rom>, Box<dyn std::error::Error>> {
        let urls = self.list_rom_urls(system, subdir).await?;

        if !self.options.latest_revision {
            // If not filtering for latest revision, just return all ROMs
            return Ok(urls
                .into_iter()
                .map(|url| {
                    let url_obj = Url::parse(&url).unwrap();
                    let path = url_obj.path();
                    let decoded = urlencoding::decode(path).expect("UTF-8");
                    let path = std::path::Path::new(decoded.as_ref());
                    let filename = path.file_name().unwrap().to_string_lossy().to_string();
                    Rom { filename, url }
                })
                .collect());
        }

        // Group ROMs by base name using BTreeMap for automatic sorting
        let mut rom_groups: BTreeMap<String, Vec<Rom>> = BTreeMap::new();

        for url in urls {
            let url_obj = Url::parse(&url).unwrap();
            let path = url_obj.path();
            let decoded = urlencoding::decode(path).expect("UTF-8");
            let path = std::path::Path::new(decoded.as_ref());
            let filename = path.file_name().unwrap().to_string_lossy().to_string();

            let (base_name, _revision) = Self::get_base_name_and_revision(&filename);
            let rom = Rom { filename, url };

            rom_groups.entry(base_name).or_default().push(rom);
        }

        // For each group, keep only the latest revision
        let mut final_roms = Vec::new();
        for roms in rom_groups.values() {
            if roms.len() == 1 {
                final_roms.push(roms[0].clone());
            } else {
                let latest = roms.iter().max_by_key(|rom| {
                    let (_, revision) = Self::get_base_name_and_revision(&rom.filename);
                    revision.unwrap_or(-1)
                });
                if let Some(rom) = latest {
                    final_roms.push(rom.clone());
                }
            }
        }

        Ok(final_roms)
    }

    fn is_valid_file(&self, href: &str) -> bool {
        let file_name = urlencoding::decode(href.split('/').last().unwrap_or(""))
            .unwrap_or_default()
            .into_owned();

        // Helper function to extract terms in parentheses
        fn get_terms_in_parentheses(filename: &str) -> Vec<String> {
            let mut terms = Vec::new();
            let mut current_term = String::new();
            let mut in_parentheses = false;

            for c in filename.chars() {
                match c {
                    '(' => {
                        in_parentheses = true;
                        current_term.clear();
                    }
                    ')' => {
                        if in_parentheses {
                            terms.push(current_term.clone());
                            in_parentheses = false;
                        }
                    }
                    _ if in_parentheses => {
                        current_term.push(c);
                    }
                    _ => {}
                }
            }
            terms
        }

        // Get all terms in parentheses
        let terms = get_terms_in_parentheses(&file_name);

        // Check region first
        if self.options.region_limit {
            let regions = [&self.options.region, "World"];
            if !terms.iter().any(|term| regions.contains(&term.as_str())) {
                return false;
            }
        }

        // Check excluded patterns
        if terms.iter().any(|term| {
            self.options
                .exclude_patterns
                .iter()
                .any(|pattern| term.contains(pattern))
        }) {
            return false;
        }

        // Check smart filters last
        if self.options.smart_filters {
            let excluded_keywords = [
                "Beta",
                "Alpha",
                "Proto",
                "Virtual Console",
                "Aftermarket",
                "Unl",
                "Sample",
                "Promo",
                "Demo",
                "Kiosk",
                // Exclude Arcade releases, some console games for some reason have an alternate Arcade ROM
                // Such as the Addams Family (1992) for SNES
                "Arcade",
            ];
            if terms
                .iter()
                .any(|term| excluded_keywords.contains(&term.as_str()))
            {
                return false;
            }
        }

        true
    }

    fn get_base_name_and_revision(filename: &str) -> (String, Option<i32>) {
        // Match everything up to the last sequence of metadata parentheses
        // Uses negative lookahead to ensure we don't stop at parentheses that are part of the name
        let re = regex::Regex::new(
            r"^(.*?)(?:\s*\([^)]*(?:Rev\s*\d+|USA|Europe|World|Japan)[^)]*\))*(?:\s*\(Rev\s*(\d+)\))?(?:\s*\([^)]*\))*(?:\..*)?$"
        ).unwrap();

        if let Some(caps) = re.captures(filename) {
            let base_name = caps.get(1).map_or("", |m| m.as_str()).trim().to_string();
            let revision = caps
                .get(2)
                .and_then(|m| m.as_str().parse::<i32>().ok())
                .or_else(|| {
                    // Fallback: look for revision number in any parentheses
                    let rev_re = regex::Regex::new(r"\(Rev\s*(\d+)\)").unwrap();
                    rev_re
                        .captures(filename)
                        .and_then(|caps| caps.get(1))
                        .and_then(|m| m.as_str().parse::<i32>().ok())
                });
            (base_name, revision)
        } else {
            // If the regex fails completely, return the whole filename
            // This should rarely happen given the pattern
            (filename.to_string(), None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_file() {
        let options = FilterOptions {
            region_limit: true,
            region: "Europe".to_string(),
            smart_filters: true,
            exclude_patterns: vec!["Beta".to_string(), "Rev B".to_string()],
            latest_revision: true,
        };

        let rom_lister = RomLister::new(options);

        assert!(rom_lister.is_valid_file("Super Game (Europe).zip"));
        assert!(rom_lister.is_valid_file("Super Game (World).zip"));
        assert!(!rom_lister.is_valid_file("Super Game (USA).zip"));
        assert!(!rom_lister.is_valid_file("Super Game (Beta).zip"));
        assert!(!rom_lister.is_valid_file("Super Game (Rev B).zip"));
        assert!(rom_lister.is_valid_file("Beta Game (Europe).zip")); // Should pass as Beta is not in parentheses
    }

    #[test]
    fn test_get_base_name_and_revision() {
        let test_cases = vec![
            ("Super Game (USA).zip", ("Super Game", None)),
            ("Super Game (Rev 2) (USA).zip", ("Super Game", Some(2))),
            ("Super Game (Rev 1).zip", ("Super Game", Some(1))),
            ("Game (Rev 12) (USA).zip", ("Game", Some(12))),
        ];

        for (input, expected) in test_cases {
            let (base, rev) = RomLister::get_base_name_and_revision(input);
            assert_eq!((base.as_str(), rev), expected);
        }
    }

    #[test]
    fn test_get_base_name_and_revision_complex() {
        let test_cases = vec![
            ("Game (World) (Legacy Game Collection).zip", ("Game", None)),
            (
                "Game (Legacy Collection) (US) (Rev 1).zip",
                ("Game", Some(1)),
            ),
            (
                "Game (Rev 2) (Legacy Collection) (World).zip",
                ("Game", Some(2)),
            ),
            (
                "Game (World) (Rev 1) (Legacy Collection).zip",
                ("Game", Some(1)),
            ),
            // Edge cases
            (
                "Game with (Parentheses) in Name (World) (Rev 3).zip",
                ("Game with (Parentheses) in Name", Some(3)),
            ),
            (
                "Game (Collection Edition) (Rev 1) (US) (Reprint).zip",
                ("Game", Some(1)),
            ),
        ];

        for (input, expected) in test_cases {
            let (base, rev) = RomLister::get_base_name_and_revision(input);
            assert_eq!(
                (base.as_str(), rev),
                expected,
                "Failed for input: {}",
                input
            );
        }
    }

    #[test]
    fn test_is_valid_file_exclusions() {
        let options = FilterOptions {
            region_limit: true,
            region: "USA".to_string(),
            smart_filters: true,
            exclude_patterns: vec!["Rental".to_string(), "Alt".to_string()],
            latest_revision: true,
        };

        let rom_lister = RomLister::new(options);

        // Region filtering
        assert!(rom_lister.is_valid_file("Game (USA).zip"));
        assert!(rom_lister.is_valid_file("Game (World).zip"));
        assert!(!rom_lister.is_valid_file("Game (Europe).zip"));
        assert!(!rom_lister.is_valid_file("Game (Japan).zip"));

        // Smart filters
        assert!(!rom_lister.is_valid_file("Game (USA) (Beta).zip"));
        assert!(!rom_lister.is_valid_file("Game (USA) (Proto).zip"));
        assert!(!rom_lister.is_valid_file("Game (USA) (Sample).zip"));
        assert!(!rom_lister.is_valid_file("Game (USA) (Demo).zip"));
        assert!(!rom_lister.is_valid_file("Game (USA) (Kiosk).zip"));
        assert!(!rom_lister.is_valid_file("Game (USA) (Unl).zip"));

        // Custom exclude patterns
        assert!(!rom_lister.is_valid_file("Game (USA) (Rental Version).zip"));
        assert!(!rom_lister.is_valid_file("Game (USA) (Alt Version).zip"));

        // Complex combinations
        assert!(!rom_lister.is_valid_file("Game (Beta) (USA) (Rev 1).zip")); // Smart filter should catch this
        assert!(!rom_lister.is_valid_file("Game (Rental) (World) (Rev 2).zip")); // Custom pattern should catch this
        assert!(!rom_lister.is_valid_file("Game (Europe) (Rev 1) (Demo).zip")); // Region and smart filter both invalid

        // These should pass
        assert!(rom_lister.is_valid_file("Game (Rev 2) (USA).zip"));
        assert!(rom_lister.is_valid_file("Game with Beta in Title (USA).zip")); // Beta not in parentheses
        assert!(rom_lister.is_valid_file("Alternative Game (USA).zip")); // Alt not in parentheses
        assert!(rom_lister.is_valid_file("Game (World) (Rev 1).zip"));
    }
}
