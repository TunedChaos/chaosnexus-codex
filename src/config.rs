// chaosnexus-codex/src/config.rs
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Configuration for a specific documentation library repository.
/// Specifies how to fetch and where to store the documentation content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryConfig {
    /// The URL of the git repository containing the documentation.
    pub repo_url: String,
    /// The specific subdirectory within the repository to fetch.
    pub sub_dir: String,
    /// The destination folder name where the documentation will be stored.
    pub dst_folder: String,
    /// Whether to use a sparse checkout (useful for large repositories).
    pub use_sparse: bool,
}

/// The root configuration for the ChaosNexus Codex application.
/// Controls server settings, reading defaults, and the list of libraries.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexConfig {
    /// Optional HTTP server port. If set, runs as an SSE HTTP server.
    pub port: Option<u16>,
    /// Default character offset when reading documentation pages.
    pub default_offset: Option<usize>,
    /// Default maximum character limit when reading documentation pages.
    pub default_limit: Option<usize>,
    /// The root directory where fetched documentation data is stored.
    pub storage_path: Option<PathBuf>,
    /// Whether to use a shared library or a local instance library.
    pub library_type: Option<String>,
    /// A list of documentation libraries to manage and serve.
    #[serde(default)]
    pub libraries: Vec<LibraryConfig>,
}

impl CodexConfig {
    pub fn merge(&mut self, other: Self) {
        if other.port.is_some() { self.port = other.port; }
        if other.default_offset.is_some() { self.default_offset = other.default_offset; }
        if other.default_limit.is_some() { self.default_limit = other.default_limit; }
        if other.storage_path.is_some() { self.storage_path = other.storage_path; }
        if other.library_type.is_some() { self.library_type = other.library_type; }
        if !other.libraries.is_empty() {
            self.libraries = other.libraries;
        }
    }

    /// Returns the resolved storage path for documentation data.
    pub fn resolved_storage_path(&self) -> PathBuf {
        if let Some(path) = &self.storage_path {
            return path.clone();
        }
        
        let base = directories::BaseDirs::new()
            .map(|d| d.home_dir().join(".chaosnexus").join("chaosnexus-codex"))
            .unwrap_or_else(|| PathBuf::from("."));

        if self.library_type.as_deref() == Some("local")
            && let Ok(instance_name) = std::env::var("CHAOS_INSTANCE_NAME") {
                return base.join(&instance_name).join("data");
            }

        base.join("data")
    }

    /// Loads the configuration hierarchically matching chaosnexus-anvil's model.
    pub fn load() -> Self {
        let mut final_config = Self::default();

        if let Some(base_dirs) = directories::BaseDirs::new() {
            let home = base_dirs.home_dir();
            let candidate_paths = [
                home.join(".chaosnexus").join("codex").join("chaosnexus-codex.toml"),
                home.join(".chaosnexus").join("codex").join("codex.toml"),
                home.join(".chaosnexus").join("codex").join("config.toml"),
                home.join(".chaosnexus").join("chaosnexus-codex").join("chaosnexus-codex.toml"),
                home.join(".chaosnexus").join("chaosnexus-codex").join("codex.toml"),
                home.join(".chaosnexus").join("chaosnexus-codex").join("config.toml"),
            ];
            for path in &candidate_paths {
                if path.exists() {
                    if let Ok(contents) = std::fs::read_to_string(path)
                        && let Ok(config) = toml::from_str::<CodexConfig>(&contents) {
                            final_config.merge(config);
                            break;
                        }
                }
            }

            if let Ok(instance_name) = std::env::var("CHAOS_INSTANCE_NAME") {
                let instance_paths = [
                    home.join(".chaosnexus").join("codex").join(&instance_name).join("chaosnexus-codex.toml"),
                    home.join(".chaosnexus").join("codex").join(&instance_name).join("config.toml"),
                    home.join(".chaosnexus").join("chaosnexus-codex").join(&instance_name).join("chaosnexus-codex.toml"),
                    home.join(".chaosnexus").join("chaosnexus-codex").join(&instance_name).join("config.toml"),
                ];
                for path in &instance_paths {
                    if path.exists() {
                        if let Ok(contents) = std::fs::read_to_string(path)
                            && let Ok(config) = toml::from_str::<CodexConfig>(&contents) {
                                final_config.merge(config);
                                break;
                            }
                    }
                }
            }
        }

        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let cwd_candidates = [
            cwd.join("chaosnexus-codex.toml"),
            cwd.join("codex.toml"),
            cwd.join("config.toml"),
        ];

        for path in &cwd_candidates {
            if path.exists() {
                if let Ok(contents) = std::fs::read_to_string(path)
                    && let Ok(config) = toml::from_str::<CodexConfig>(&contents) {
                        final_config.merge(config);
                        break;
                    }
            }
        }

        final_config
    }

    /// Saves the current configuration to `chaosnexus-codex.toml`.
    #[allow(dead_code)]
    pub fn save(&self) -> std::io::Result<()> {
        let config_path = Path::new("chaosnexus-codex.toml");
        let contents = toml::to_string(self).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(config_path, contents)
    }
}
