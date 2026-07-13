use crate::config::CodexConfig;
use tokio::process::Command;
use walkdir::WalkDir;

/// Fetches and flattens all configured documentation libraries.
/// Reads the library configurations, clones repositories into a temporary directory
/// (optionally using sparse checkouts), and copies all markdown files into a flattened
/// structure within the designated storage path.
pub async fn fetch_all_docs() -> Result<(), Box<dyn std::error::Error>> {
    let config = CodexConfig::load();
    let data_dir = config.resolved_storage_path();
    let tmp_dir = std::env::temp_dir().join("chaosdocs_docs_fetch");

    if tmp_dir.exists() {
        std::fs::remove_dir_all(&tmp_dir)?;
    }
    std::fs::create_dir_all(&tmp_dir)?;
    std::fs::create_dir_all(&data_dir)?;

    for lib in config.libraries {
        tracing::info!("Fetching {} Documentation...", lib.dst_folder);

        let tmp_repo = tmp_dir.join(&lib.dst_folder);
        let dst_dir = data_dir.join(&lib.dst_folder);

        if tmp_repo.exists() {
            std::fs::remove_dir_all(&tmp_repo)?;
        }
        std::fs::create_dir_all(&dst_dir)?;

        let mut cmd = Command::new("git");
        if lib.use_sparse {
            cmd.arg("clone")
                .arg("--depth")
                .arg("1")
                .arg("--filter=blob:none")
                .arg("--sparse")
                .arg(&lib.repo_url)
                .arg(&tmp_repo);
        } else {
            cmd.arg("clone")
                .arg("--depth")
                .arg("1")
                .arg(&lib.repo_url)
                .arg(&tmp_repo);
        }

        let status = cmd.status().await?;
        if !status.success() {
            tracing::warn!("⚠️ Failed to clone {}", lib.repo_url);
            continue;
        }

        if lib.use_sparse && !lib.sub_dir.is_empty() && lib.sub_dir != "." && lib.sub_dir != "/" {
            let mut sparse_cmd = Command::new("git");
            sparse_cmd.current_dir(&tmp_repo)
                .arg("sparse-checkout")
                .arg("set")
                .arg(&lib.sub_dir);
            let sparse_status = sparse_cmd.status().await?;
            if !sparse_status.success() {
                tracing::warn!("⚠️ Failed to set sparse-checkout for {} to {}", lib.repo_url, lib.sub_dir);
            }
        }

        let src_dir = tmp_repo.join(&lib.sub_dir);
        if !src_dir.exists() {
            tracing::warn!("⚠️ Source directory {:?} not found. Skipping.", src_dir);
            continue;
        }

        let mut count = 0;
        for entry in WalkDir::new(&src_dir) {
            let Ok(entry) = entry else { continue; };

            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "md" || ext == "mdx" {
                let rel_path = path.strip_prefix(&src_dir)?;
                let flat_name = rel_path.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "_");
                let dst_path = dst_dir.join(flat_name);
                
                std::fs::copy(path, dst_path)?;
                count += 1;
            }
        }
        
        tracing::info!("✅ Fetched and flattened {} files for {}.", count, lib.dst_folder);
    }

    tracing::info!("Cleaning up...");
    std::fs::remove_dir_all(&tmp_dir)?;
    tracing::info!("Done! Restart chaosnexus-codex (without embed-docs) for changes to take effect.");

    Ok(())
}
