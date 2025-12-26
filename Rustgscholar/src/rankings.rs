//! EasyScholar publication rankings API client.
//!
//! This module provides access to EasyScholar's ranking data,
//! including Impact Factor (IF), JCI, and SCI partitions.

use crate::error::{GscholarError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// EasyScholar API base URL
const EASYSCHOLAR_API_URL: &str = "https://www.easyscholar.cc/open/getPublicationRank";

/// Minimum interval between requests (slightly more than 0.5s to be safe)
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(600);

/// Ranking metrics from EasyScholar
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RankingMetrics {
    /// Impact Factor
    pub sciif: Option<String>,
    /// Journal Citation Indicator
    pub jci: Option<String>,
    /// SCI partition (Q1, Q2, etc.)
    pub sci: Option<String>,
    /// SCI Up Top
    pub sci_up_top: Option<String>,
    /// SCI Base
    pub sci_base: Option<String>,
    /// SCI Up
    pub sci_up: Option<String>,
}

/// EasyScholar API client with caching and rate limiting
pub struct RankingClient {
    secret_key: String,
    client: reqwest::Client,
    cache: Mutex<HashMap<String, Option<RankingMetrics>>>,
    last_request: Mutex<Option<Instant>>,
}

impl RankingClient {
    /// Create a new RankingClient
    ///
    /// # Arguments
    ///
    /// * `secret_key` - EasyScholar API key
    pub fn new(secret_key: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| GscholarError::Config(format!("Failed to build HTTP client: {}", e)))?;

        Ok(Self {
            secret_key,
            client,
            cache: Mutex::new(HashMap::new()),
            last_request: Mutex::new(None),
        })
    }

    /// Get ranking info for a journal/venue
    ///
    /// Returns None if not found or error
    pub async fn get_rank(&self, venue_name: &str) -> Option<RankingMetrics> {
        let venue_name = venue_name.trim();
        if venue_name.is_empty() {
            return None;
        }

        // Check cache first
        {
            let cache = self.cache.lock().ok()?;
            if let Some(cached) = cache.get(venue_name) {
                info!(venue = venue_name, "Cache hit");
                return cached.clone();
            }
        }

        // Rate limiting
        self.wait_for_rate_limit().await;

        // Make request
        let result = self.do_request(venue_name).await;

        // Note: The block below handles caching properly
        // Update cache
        {
            if let Ok(mut cache) = self.cache.lock() {
                cache.insert(venue_name.to_string(), result.clone());
            }
        }

        result
    }

    /// Wait for rate limit interval
    async fn wait_for_rate_limit(&self) {
        let should_wait = {
            let last = self.last_request.lock().ok();
            last.and_then(|l| *l).map(|t| t.elapsed() < MIN_REQUEST_INTERVAL)
        };

        if should_wait == Some(true) {
            tokio::time::sleep(MIN_REQUEST_INTERVAL).await;
        }

        // Update last request time
        if let Ok(mut last) = self.last_request.lock() {
            *last = Some(Instant::now());
        }
    }

    /// Internal request implementation
    async fn do_request(&self, venue_name: &str) -> Option<RankingMetrics> {
        debug!(venue = venue_name, "Querying EasyScholar");

        let response = self
            .client
            .get(EASYSCHOLAR_API_URL)
            .query(&[
                ("secretKey", self.secret_key.as_str()),
                ("publicationName", venue_name),
            ])
            .send()
            .await
            .ok()?;

        if !response.status().is_success() {
            warn!(
                venue = venue_name,
                status = response.status().as_u16(),
                "EasyScholar API error"
            );
            return None;
        }

        let data: EasyScholarResponse = match response.json().await {
            Ok(d) => d,
            Err(e) => {
                warn!(venue = venue_name, error = %e, "Failed to parse response");
                return None;
            }
        };

        if data.code != 200 {
            warn!(
                venue = venue_name,
                code = data.code,
                msg = data.msg.as_deref().unwrap_or("Unknown"),
                "EasyScholar API returned error"
            );
            return None;
        }

        let result = data.data.map(|d| extract_metrics(&d));
        
        if result.is_some() {
            info!(venue = venue_name, "Found ranking data");
        } else {
            debug!(venue = venue_name, "No ranking data found");
        }
        
        result
    }

    /// Get a specific metric from ranking data
    ///
    /// # Arguments
    ///
    /// * `metrics` - Ranking metrics
    /// * `key` - Metric key ("sciif", "jci", "sci", etc.)
    pub fn get_metric(metrics: &RankingMetrics, key: &str) -> Option<String> {
        match key {
            "sciif" => metrics.sciif.clone(),
            "jci" => metrics.jci.clone(),
            "sci" => metrics.sci.clone(),
            "sciUpTop" => metrics.sci_up_top.clone(),
            "sciBase" => metrics.sci_base.clone(),
            "sciUp" => metrics.sci_up.clone(),
            _ => None,
        }
    }

    /// Check if a value passes a numeric filter
    ///
    /// # Arguments
    ///
    /// * `value` - The metric value
    /// * `threshold` - Minimum threshold
    pub fn passes_numeric_filter(value: Option<&str>, threshold: f64) -> bool {
        value
            .and_then(|v| v.parse::<f64>().ok())
            .map(|v| v >= threshold)
            .unwrap_or(false)
    }

    /// Check if a value passes a substring filter
    ///
    /// # Arguments
    ///
    /// * `value` - The metric value
    /// * `pattern` - Substring to match
    pub fn passes_string_filter(value: Option<&str>, pattern: &str) -> bool {
        value
            .map(|v| v.contains(pattern))
            .unwrap_or(false)
    }
}

// === EasyScholar API Response Types ===

#[derive(Debug, Deserialize)]
struct EasyScholarResponse {
    code: i32,
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    data: Option<EasyScholarData>,
}

#[derive(Debug, Deserialize)]
struct EasyScholarData {
    #[serde(rename = "officialRank", default)]
    official_rank: Option<OfficialRank>,
}

#[derive(Debug, Deserialize)]
struct OfficialRank {
    #[serde(default)]
    select: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    all: Option<HashMap<String, serde_json::Value>>,
}

/// Extract metrics from EasyScholar response data
fn extract_metrics(data: &EasyScholarData) -> RankingMetrics {
    let mut metrics = RankingMetrics::default();

    if let Some(ref official) = data.official_rank {
        // Try `select` first, then `all`
        let select = official.select.as_ref();
        let all = official.all.as_ref();

        metrics.sciif = get_value(select, all, "sciif");
        metrics.jci = get_value(select, all, "jci");
        metrics.sci = get_value(select, all, "sci");
        metrics.sci_up_top = get_value(select, all, "sciUpTop");
        metrics.sci_base = get_value(select, all, "sciBase");
        metrics.sci_up = get_value(select, all, "sciUp");
    }

    metrics
}

/// Get value from select or all maps
fn get_value(
    select: Option<&HashMap<String, serde_json::Value>>,
    all: Option<&HashMap<String, serde_json::Value>>,
    key: &str,
) -> Option<String> {
    // Try select first
    if let Some(map) = select {
        if let Some(val) = map.get(key) {
            return value_to_string(val);
        }
    }

    // Fall back to all
    if let Some(map) = all {
        if let Some(val) = map.get(key) {
            return value_to_string(val);
        }
    }

    None
}

/// Convert JSON value to string
fn value_to_string(val: &serde_json::Value) -> Option<String> {
    match val {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Null => None,
        _ => Some(val.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_passes_numeric_filter() {
        assert!(RankingClient::passes_numeric_filter(Some("5.5"), 5.0));
        assert!(!RankingClient::passes_numeric_filter(Some("4.9"), 5.0));
        assert!(!RankingClient::passes_numeric_filter(None, 5.0));
        assert!(!RankingClient::passes_numeric_filter(Some("invalid"), 5.0));
    }

    #[test]
    fn test_passes_string_filter() {
        assert!(RankingClient::passes_string_filter(Some("Q1"), "Q1"));
        assert!(RankingClient::passes_string_filter(Some("Q1/Q2"), "Q1"));
        assert!(!RankingClient::passes_string_filter(Some("Q2"), "Q1"));
        assert!(!RankingClient::passes_string_filter(None, "Q1"));
    }
}
