use async_trait::async_trait;
use crate::core::{CommandExecutor, Package, PackageManager, RateLimiter, Result, Error};
use once_cell::sync::OnceCell;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubPackage {
    pub owner: String,
    pub repo: String,
    pub version: Option<String>,
    pub asset_pattern: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    #[allow(dead_code)]
    size: u64,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InstalledGithubPackage {
    owner: String,
    repo: String,
    version: String,
}

pub struct GithubManager {
    executor: CommandExecutor,
    #[allow(dead_code)]
    available: OnceCell<bool>,
    client: Client,
    rate_limiter: RateLimiter,
    install_dir: PathBuf,
    state_file: PathBuf,
    token: Option<String>,
}

impl GithubManager {
    pub fn new(executor: CommandExecutor) -> Self {
        Self::with_config(executor, None)
    }

    pub fn with_config(executor: CommandExecutor, token: Option<String>) -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let linix_dir = PathBuf::from(&home).join(".local").join("share").join("linix");

        Self {
            executor,
            available: OnceCell::new(),
            client: Client::new(),
            rate_limiter: if token.is_some() {
                RateLimiter::github_authenticated()
            } else {
                RateLimiter::github()
            },
            install_dir: linix_dir.join("github"),
            state_file: linix_dir.join("github_installed.json"),
            token,
        }
    }

    pub fn parse_github_url(spec: &str) -> Option<GithubPackage> {
        let spec = spec
            .trim_start_matches("https://github.com/")
            .trim_start_matches("github:")
            .trim_end_matches('/');

        let (spec_without_pattern, asset_pattern) = if spec.contains('#') {
            let parts: Vec<&str> = spec.splitn(2, '#').collect();
            (parts[0], Some(parts[1].to_string()))
        } else {
            (spec, None)
        };

        let (spec_without_version, version) = if spec_without_pattern.contains('@') {
            let parts: Vec<&str> = spec_without_pattern.splitn(2, '@').collect();
            (parts[0], Some(parts[1].to_string()))
        } else {
            (spec_without_pattern, None)
        };

        let parts: Vec<&str> = spec_without_version.split('/').collect();
        if parts.len() >= 2 {
            Some(GithubPackage {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
                version,
                asset_pattern,
            })
        } else {
            None
        }
    }

    async fn get_latest_release(&self, owner: &str, repo: &str) -> Result<GithubRelease> {
        self.rate_limiter.wait().await?;

        let url = format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            owner, repo
        );

        let mut request = self.client.get(&url)
            .header("User-Agent", "linix-package-manager")
            .header("Accept", "application/vnd.github.v3+json");

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("token {}", token));
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(Error::Http(response.error_for_status().unwrap_err()));
        }

        let release: GithubRelease = response.json().await?;
        Ok(release)
    }

    fn find_best_asset<'a>(&self, assets: &'a [GithubAsset], pattern: Option<&str>) -> Option<&'a GithubAsset> {
        let arch = std::env::consts::ARCH;
        let os = std::env::consts::OS;

        let arch_patterns: Vec<&str> = match arch {
            "x86_64" => vec!["x86_64", "amd64", "x64", "linux64"],
            "aarch64" => vec!["aarch64", "arm64"],
            "x86" => vec!["x86", "i386", "i686", "386"],
            _ => vec![arch],
        };

        let os_patterns: Vec<&str> = match os {
            "linux" => vec!["linux", "Linux"],
            "macos" => vec!["darwin", "macos", "osx", "Darwin"],
            "windows" => vec!["windows", "Windows", "win"],
            _ => vec![os],
        };

        let filtered_assets: Vec<&GithubAsset> = if let Some(pat) = pattern {
            let regex = Regex::new(pat).ok();
            assets
                .iter()
                .filter(|a| {
                    regex.as_ref().map(|r| r.is_match(&a.name)).unwrap_or(false)
                        || a.name.contains(pat)
                })
                .collect()
        } else {
            assets.iter().collect()
        };

        let mut best_asset: Option<(&GithubAsset, i32)> = None;

        for asset in filtered_assets {
            let name_lower = asset.name.to_lowercase();
            let mut score = 0;

            for os_pat in &os_patterns {
                if name_lower.contains(&os_pat.to_lowercase()) {
                    score += 10;
                    break;
                }
            }

            for arch_pat in &arch_patterns {
                if name_lower.contains(&arch_pat.to_lowercase()) {
                    score += 10;
                    break;
                }
            }

            if name_lower.ends_with(".deb") && os == "linux" {
                score += 5;
            } else if name_lower.ends_with(".rpm") && os == "linux" {
                score += 3;
            } else if name_lower.ends_with(".appimage") {
                score += 4;
            } else if name_lower.ends_with(".tar.gz") || name_lower.ends_with(".tgz") {
                score += 2;
            } else if name_lower.ends_with(".zip") {
                score += 1;
            }

            if name_lower.contains("source") || name_lower.contains("src") {
                score -= 20;
            }

            if best_asset.is_none() || score > best_asset.unwrap().1 {
                best_asset = Some((asset, score));
            }
        }

        best_asset.map(|(a, _)| a)
    }

    async fn download_file(&self, url: &str, dest: &Path) -> Result<()> {
        info!("Downloading: {}", url);

        let mut request = self.client.get(url).header("User-Agent", "linix-package-manager");

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("token {}", token));
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(Error::Http(response.error_for_status().unwrap_err()));
        }

        let bytes = response.bytes().await?;
        fs::write(dest, &bytes)?;

        Ok(())
    }

    async fn install_single(&self, pkg: &GithubPackage, sudo: bool) -> Result<()> {
        info!("Installing {}/{}", pkg.owner, pkg.repo);

        let release = self.get_latest_release(&pkg.owner, &pkg.repo).await?;
        debug!("Found release: {}", release.tag_name);

        let asset = self
            .find_best_asset(&release.assets, pkg.asset_pattern.as_deref())
            .ok_or_else(|| Error::PackageNotFound(format!(
                "No suitable asset found for {}/{}",
                pkg.owner, pkg.repo
            )))?;

        debug!("Selected asset: {}", asset.name);

        let temp_dir = tempfile::tempdir()?;
        let download_path = temp_dir.path().join(&asset.name);

        self.download_file(&asset.browser_download_url, &download_path).await?;

        let name_lower = asset.name.to_lowercase();

        if name_lower.ends_with(".deb") {
            self.executor.run("dpkg", &["-i", download_path.to_str().unwrap()], sudo).await?;
        } else if name_lower.ends_with(".rpm") {
            self.executor.run("rpm", &["-i", download_path.to_str().unwrap()], sudo).await?;
        } else if name_lower.ends_with(".appimage") {
            self.install_appimage(&download_path, &pkg.repo).await?;
        } else if name_lower.ends_with(".tar.gz") || name_lower.ends_with(".tgz") || name_lower.ends_with(".tar.xz") {
            self.install_tarball(&download_path, &pkg.repo).await?;
        } else if name_lower.ends_with(".zip") {
            self.install_zip(&download_path, &pkg.repo).await?;
        } else {
            self.install_binary(&download_path, &pkg.repo).await?;
        }

        self.save_installed_package(pkg, &release.tag_name).await?;

        info!("Successfully installed {}/{} {}", pkg.owner, pkg.repo, release.tag_name);
        Ok(())
    }

    async fn install_appimage(&self, path: &Path, name: &str) -> Result<()> {
        let dest = self.install_dir.join(format!("{}.AppImage", name));
        fs::create_dir_all(&self.install_dir)?;
        fs::copy(path, &dest)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))?;
        }

        let bin_dir = PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".local")
            .join("bin");
        fs::create_dir_all(&bin_dir)?;

        let link_path = bin_dir.join(name);
        let _ = fs::remove_file(&link_path);

        #[cfg(unix)]
        std::os::unix::fs::symlink(&dest, &link_path)?;

        Ok(())
    }

    async fn install_tarball(&self, path: &Path, name: &str) -> Result<()> {
        let extract_dir = self.install_dir.join(name);
        fs::create_dir_all(&extract_dir)?;

        self.executor
            .run("tar", &[
                "-xf",
                path.to_str().unwrap(),
                "-C",
                extract_dir.to_str().unwrap(),
                "--strip-components=1",
            ], false)
            .await?;

        self.symlink_binaries(&extract_dir, name).await?;

        Ok(())
    }

    async fn install_zip(&self, path: &Path, name: &str) -> Result<()> {
        let extract_dir = self.install_dir.join(name);
        fs::create_dir_all(&extract_dir)?;

        self.executor
            .run("unzip", &[
                "-o",
                path.to_str().unwrap(),
                "-d",
                extract_dir.to_str().unwrap(),
            ], false)
            .await?;

        self.symlink_binaries(&extract_dir, name).await?;

        Ok(())
    }

    async fn install_binary(&self, path: &Path, name: &str) -> Result<()> {
        let bin_dir = PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".local")
            .join("bin");
        fs::create_dir_all(&bin_dir)?;

        let dest = bin_dir.join(name);
        fs::copy(path, &dest)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dest, fs::Permissions::from_mode(0o755))?;
        }

        Ok(())
    }

    async fn symlink_binaries(&self, dir: &Path, _name: &str) -> Result<()> {
        let bin_dir = PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".local")
            .join("bin");
        fs::create_dir_all(&bin_dir)?;

        for entry in walkdir::WalkDir::new(dir).max_depth(3).into_iter().filter_map(|e| e.ok()) {
            let entry_path = entry.path();

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if entry_path.is_file() {
                    if let Ok(metadata) = entry_path.metadata() {
                        if metadata.permissions().mode() & 0o111 != 0 {
                            if let Some(file_name) = entry_path.file_name() {
                                let link_path = bin_dir.join(file_name);
                                let _ = fs::remove_file(&link_path);
                                std::os::unix::fs::symlink(entry_path, &link_path)?;
                            }
                        }
                    }
                }
            }

            #[cfg(not(unix))]
            {
                let _ = entry_path;
            }
        }

        Ok(())
    }

    fn load_installed_state(&self) -> HashMap<String, InstalledGithubPackage> {
        if self.state_file.exists() {
            let content = fs::read_to_string(&self.state_file).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            HashMap::new()
        }
    }

    async fn save_installed_package(&self, pkg: &GithubPackage, version: &str) -> Result<()> {
        let mut state = self.load_installed_state();

        let key = format!("{}/{}", pkg.owner, pkg.repo);
        state.insert(key, InstalledGithubPackage {
            owner: pkg.owner.clone(),
            repo: pkg.repo.clone(),
            version: version.to_string(),
        });

        fs::create_dir_all(self.state_file.parent().unwrap())?;
        let content = serde_json::to_string_pretty(&state)?;
        fs::write(&self.state_file, content)?;

        Ok(())
    }

    fn remove_from_state(&self, owner: &str, repo: &str) -> Result<()> {
        let mut state = self.load_installed_state();
        let key = format!("{}/{}", owner, repo);
        state.remove(&key);

        let content = serde_json::to_string_pretty(&state)?;
        fs::write(&self.state_file, content)?;

        Ok(())
    }
}

#[async_trait]
impl PackageManager for GithubManager {
    fn name(&self) -> &str {
        "github"
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn install(&self, packages: &[String], sudo: bool) -> Result<()> {
        for spec in packages {
            let pkg = Self::parse_github_url(spec)
                .ok_or_else(|| Error::Parse(format!("Invalid GitHub spec: {}", spec)))?;

            self.install_single(&pkg, sudo).await?;
        }
        Ok(())
    }

    async fn remove(&self, packages: &[String], _sudo: bool) -> Result<()> {
        for spec in packages {
            let pkg = Self::parse_github_url(spec)
                .ok_or_else(|| Error::Parse(format!("Invalid GitHub spec: {}", spec)))?;

            let bin_dir = PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".local")
                .join("bin");

            let binary_path = bin_dir.join(&pkg.repo);
            if binary_path.exists() {
                fs::remove_file(&binary_path).ok();
            }

            let install_path = self.install_dir.join(&pkg.repo);
            if install_path.exists() {
                fs::remove_dir_all(&install_path).ok();
            }

            let appimage_path = self.install_dir.join(format!("{}.AppImage", pkg.repo));
            if appimage_path.exists() {
                fs::remove_file(&appimage_path).ok();
            }

            self.remove_from_state(&pkg.owner, &pkg.repo)?;

            info!("Removed {}/{}", pkg.owner, pkg.repo);
        }

        Ok(())
    }

    async fn list_installed(&self) -> Result<Vec<Package>> {
        let state = self.load_installed_state();

        let packages = state
            .into_iter()
            .map(|(_, pkg)| Package {
                name: format!("{}/{}", pkg.owner, pkg.repo),
                version: Some(pkg.version),
                backend: self.name().to_string(),
                description: None,
                repository: Some(format!("https://github.com/{}/{}", pkg.owner, pkg.repo)),
                size: None,
            })
            .collect();

        Ok(packages)
    }

    async fn search(&self, query: &str) -> Result<Vec<Package>> {
        self.rate_limiter.wait().await?;

        let url = format!(
            "https://api.github.com/search/repositories?q={}+in:name&per_page=20",
            query
        );

        let mut request = self.client.get(&url)
            .header("User-Agent", "linix-package-manager")
            .header("Accept", "application/vnd.github.v3+json");

        if let Some(token) = &self.token {
            request = request.header("Authorization", format!("token {}", token));
        }

        let response = request.send().await?;
        let json: serde_json::Value = response.json().await?;

        let packages = json
            .get("items")
            .and_then(|items| items.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let name = item.get("full_name")?.as_str()?.to_string();
                        let description = item.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
                        let url = item.get("html_url").and_then(|u| u.as_str()).map(|s| s.to_string());

                        Some(Package {
                            name,
                            version: None,
                            backend: self.name().to_string(),
                            description,
                            repository: url,
                            size: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(packages)
    }

    fn supports_orphan_cleanup(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_url() {
        let pkg = GithubManager::parse_github_url("owner/repo").unwrap();
        assert_eq!(pkg.owner, "owner");
        assert_eq!(pkg.repo, "repo");
        assert!(pkg.version.is_none());

        let pkg = GithubManager::parse_github_url("owner/repo@v1.0.0").unwrap();
        assert_eq!(pkg.version, Some("v1.0.0".to_string()));

        let pkg = GithubManager::parse_github_url("https://github.com/owner/repo").unwrap();
        assert_eq!(pkg.owner, "owner");
        assert_eq!(pkg.repo, "repo");

        let pkg = GithubManager::parse_github_url("owner/repo#linux.*amd64").unwrap();
        assert_eq!(pkg.asset_pattern, Some("linux.*amd64".to_string()));
    }
}
