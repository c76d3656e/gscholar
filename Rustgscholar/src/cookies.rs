//! Cookie management for Google Scholar requests.
//!
//! This module handles cookie persistence to maintain session state
//! and avoid rate limiting from Google Scholar.

use crate::error::{GscholarError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::{debug, info, warn};

/// Default cookie file path: `~/.gscholar_cookies.json`
fn default_cookie_path() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|p| p.join(".gscholar_cookies.json"))
        .ok_or_else(|| GscholarError::Config("Cannot determine home directory".to_string()))
}

/// Cookie entry matching Playwright's cookie format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub http_only: bool,
    #[serde(default)]
    pub expires: Option<f64>,
}

/// Cookie manager for loading and saving cookies
pub struct CookieManager {
    path: PathBuf,
}

impl CookieManager {
    /// Create a new CookieManager with default path
    pub fn new() -> Result<Self> {
        Ok(Self {
            path: default_cookie_path()?,
        })
    }

    /// Create a new CookieManager with custom path
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    /// Get the cookie file path
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Load cookies from file
    ///
    /// Returns empty vec if file doesn't exist or is invalid
    pub fn load(&self) -> Vec<Cookie> {
        if !self.path.exists() {
            debug!("Cookie file not found: {:?}", self.path);
            return Vec::new();
        }

        match std::fs::read_to_string(&self.path) {
            Ok(content) => match serde_json::from_str::<Vec<Cookie>>(&content) {
                Ok(cookies) => {
                    info!("Loaded {} cookies from {:?}", cookies.len(), self.path);
                    cookies
                }
                Err(e) => {
                    warn!("Failed to parse cookies: {}", e);
                    Vec::new()
                }
            },
            Err(e) => {
                warn!("Failed to read cookie file: {}", e);
                Vec::new()
            }
        }
    }

    /// Load cookies as a HashMap for easy lookup
    pub fn load_as_map(&self) -> HashMap<String, String> {
        self.load()
            .into_iter()
            .map(|c| (c.name, c.value))
            .collect()
    }

    /// Save cookies to file
    pub fn save(&self, cookies: &[Cookie]) -> Result<()> {
        let content = serde_json::to_string_pretty(cookies)?;
        std::fs::write(&self.path, content)?;
        info!("Saved {} cookies to {:?}", cookies.len(), self.path);
        Ok(())
    }

    /// Clear stored cookies
    pub fn clear(&self) -> Result<()> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
            info!("Cleared cookies at {:?}", self.path);
        }
        Ok(())
    }
}

impl Default for CookieManager {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| Self {
            path: PathBuf::from(".gscholar_cookies.json"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_load_empty() {
        let manager = CookieManager::with_path(PathBuf::from("/nonexistent/path"));
        assert!(manager.load().is_empty());
    }

    #[test]
    fn test_save_and_load() -> Result<()> {
        let temp = NamedTempFile::new()?;
        let manager = CookieManager::with_path(temp.path().to_path_buf());

        let cookies = vec![Cookie {
            name: "test".to_string(),
            value: "value".to_string(),
            domain: ".example.com".to_string(),
            path: "/".to_string(),
            secure: true,
            http_only: false,
            expires: None,
        }];

        manager.save(&cookies)?;
        let loaded = manager.load();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "test");
        Ok(())
    }
}
