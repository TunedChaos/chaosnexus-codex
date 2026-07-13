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

    /// Loads the configuration hierarchically.
    pub fn load() -> Self {
        let mut final_config = Self::default();

        if let Some(base_dirs) = directories::BaseDirs::new() {
            let global_config_path = base_dirs.home_dir().join(".chaosnexus").join("chaosnexus-codex").join("config.toml");
            if global_config_path.exists() {
                if let Ok(contents) = std::fs::read_to_string(&global_config_path)
                    && let Ok(config) = toml::from_str::<CodexConfig>(&contents) {
                        final_config.merge(config);
                    }
            } else {
                let boilerplate = r#"# ChaosNexus Codex Configuration
# port = 3000 # Uncomment to run as an SSE HTTP server on the given port by default
# default_offset = 0 # Default character offset for reading documentation pages
# default_limit = 16000 # Default maximum character limit for reading documentation pages
# library_type = "shared" # Or "local"

# Example: TypeScript
# [[libraries]]
# repo_url = "https://github.com/microsoft/TypeScript-Website.git"
# sub_dir = "packages/documentation/copy/en"
# dst_folder = "typescript"
# use_sparse = false
"#;
                if let Some(parent) = global_config_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&global_config_path, boilerplate);
                tracing::info!("Created boilerplate config at {:?}", global_config_path);
            }

            if let Ok(instance_name) = std::env::var("CHAOS_INSTANCE_NAME") {
                let instance_config_path = base_dirs.home_dir().join(".chaosnexus").join("chaosnexus-codex").join(&instance_name).join("config.toml");
                if instance_config_path.exists()
                    && let Ok(contents) = std::fs::read_to_string(&instance_config_path)
                        && let Ok(config) = toml::from_str::<CodexConfig>(&contents) {
                            final_config.merge(config);
                        }
            }
        }

        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let cwd_config_path = cwd.join("chaosnexus-codex.toml");

        if cwd_config_path.exists()
            && let Ok(contents) = std::fs::read_to_string(&cwd_config_path)
                && let Ok(config) = toml::from_str::<CodexConfig>(&contents) {
                    final_config.merge(config);
                }

        final_config
    }

    /// Saves the current configuration to the `chaosnexus-codex.toml` file.
    #[allow(dead_code)]
    pub fn save(&self) -> std::io::Result<()> {
        let config_path = Path::new("chaosnexus-codex.toml");
        let contents = toml::to_string(self).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(config_path, contents)
    }
}
