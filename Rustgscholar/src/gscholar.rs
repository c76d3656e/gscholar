//! Google Scholar scraping module using Playwright.
//!
//! This module provides the core scraping functionality for Google Scholar
//! using Playwright for browser automation with anti-detection features.

use crate::error::{GscholarError, Result};
use regex::Regex;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, error, info, warn};
use url::Url;

/// Default Google Scholar URL
pub const DEFAULT_SCHOLAR_URL: &str = "https://scholar.google.com";

/// User agent string for requests
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// A single search result from Google Scholar
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScholarResult {
    /// Article title
    pub title: String,
    /// Authors
    pub author: String,
    /// Publication year
    pub year: String,
    /// Journal/Conference venue
    pub venue: String,
    /// Direct URL to the article
    pub article_url: String,
    /// Number of citations
    pub citations: String,
    /// Text snippet from the article
    pub snippet: String,
}

/// Query options for Google Scholar search
#[derive(Debug, Clone)]
pub struct QueryOptions {
    /// Proxy URL (e.g., "http://127.0.0.1:7890")
    pub proxy: Option<String>,
    /// Page numbers to fetch (1-indexed)
    pub pages: Vec<i32>,
    /// Source data type filter (default: "0,5" for articles only)
    pub sdt: String,
    /// Year low filter (results from this year onwards)
    pub ylo: Option<i32>,
    /// Custom base URL for mirror sites
    pub base_url: Option<String>,
    /// Whether to return all results or just first per page
    pub all_results: bool,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            proxy: None,
            pages: vec![1],
            sdt: "0,5".to_string(),
            ylo: None,
            base_url: None,
            all_results: true,
        }
    }
}

/// Query Google Scholar and return results.
///
/// # Arguments
///
/// * `search_str` - Search query string
/// * `options` - Query options
///
/// # Returns
///
/// List of search results
///
/// # Errors
///
/// Returns error if browser fails to launch or network error occurs
pub async fn query(search_str: &str, options: &QueryOptions) -> Result<Vec<ScholarResult>> {
    let scholar_url = options
        .base_url
        .as_ref()
        .map(|s| s.trim_end_matches('/').to_string())
        .unwrap_or_else(|| DEFAULT_SCHOLAR_URL.to_string());

    info!(
        query = search_str,
        url = %scholar_url,
        pages = ?options.pages,
        "Starting Google Scholar query"
    );

    let mut all_results = Vec::new();

    // Load cookies from cookie manager
    let cookie_manager = crate::cookies::CookieManager::default();
    let cookies = cookie_manager.load();
    let cookie_header = build_cookie_header(&cookies);
    
    if cookies.is_empty() {
        warn!("No cookies loaded. Run 'rustgscholar cookies fetch' to get cookies from browser.");
    } else {
        info!("Loaded {} cookies for Google Scholar", cookies.len());
    }

    // Build HTTP client with cookies
    let client = build_http_client(options.proxy.as_deref())?;

    for page_num in &options.pages {
        let start = (page_num - 1) * 10;
        let url = build_search_url(&scholar_url, search_str, start, &options.sdt, options.ylo)?;

        debug!(page = page_num, url = %url, "Fetching page");

        // Add random delay to avoid detection
        let delay = rand::random::<u64>() % 1500 + 500;
        tokio::time::sleep(Duration::from_millis(delay)).await;

        match fetch_page_with_cookies(&client, &url, &cookie_header).await {
            Ok(html) => {
                // Check for CAPTCHA
                if html.contains("Solving the above CAPTCHA") || html.contains("unusual traffic") {
                    warn!(page = page_num, "CAPTCHA detected");
                    return Err(GscholarError::Captcha);
                }

                let page_results = parse_result_items(&html)?;
                info!(page = page_num, count = page_results.len(), "Parsed results");

                // Debug: save HTML to file if no results found (first page only)
                if page_results.is_empty() && *page_num == 1 {
                    let debug_path = std::path::Path::new("debug_gscholar.html");
                    if let Err(e) = std::fs::write(debug_path, &html) {
                        warn!("Failed to write debug HTML: {}", e);
                    } else {
                        info!("Debug HTML saved to: {:?}", debug_path);
                    }
                }

                if options.all_results {
                    all_results.extend(page_results);
                } else if let Some(first) = page_results.into_iter().next() {
                    all_results.push(first);
                }
            }
            Err(e) => {
                error!(page = page_num, error = %e, "Failed to fetch page");
                // Continue with other pages instead of failing completely
            }
        }
    }

    info!(total = all_results.len(), "Query complete");
    Ok(all_results)
}

/// Build cookie header string from cookie list
fn build_cookie_header(cookies: &[crate::cookies::Cookie]) -> String {
    cookies
        .iter()
        .filter(|c| c.domain.contains("google"))
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Build HTTP client with optional proxy
fn build_http_client(proxy: Option<&str>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(30))
        .cookie_store(true);

    if let Some(proxy_url) = proxy {
        let proxy = reqwest::Proxy::all(proxy_url).map_err(|e| {
            GscholarError::Config(format!("Invalid proxy URL '{}': {}", proxy_url, e))
        })?;
        builder = builder.proxy(proxy);
    }

    builder
        .build()
        .map_err(|e| GscholarError::Config(format!("Failed to build HTTP client: {}", e)))
}

/// Build Google Scholar search URL
fn build_search_url(
    base_url: &str,
    query: &str,
    start: i32,
    sdt: &str,
    ylo: Option<i32>,
) -> Result<Url> {
    let mut url = Url::parse(&format!("{}/scholar", base_url))
        .map_err(|e| GscholarError::Config(format!("Invalid base URL: {}", e)))?;

    {
        let mut params = url.query_pairs_mut();
        params.append_pair("q", query);
        params.append_pair("hl", "en-US");  // Force English locale for consistent parsing
        params.append_pair("start", &start.to_string());
        params.append_pair("as_sdt", sdt);
        if let Some(year) = ylo {
            params.append_pair("as_ylo", &year.to_string());
        }
    }

    Ok(url)
}

/// Fetch page content using HTTP client
async fn fetch_page(client: &reqwest::Client, url: &Url) -> Result<String> {
    fetch_page_with_cookies(client, url, "").await
}

/// Fetch page content using HTTP client with cookies
async fn fetch_page_with_cookies(client: &reqwest::Client, url: &Url, cookie_header: &str) -> Result<String> {
    let mut request = client
        .get(url.as_str())
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Cache-Control", "no-cache")
        .header("Pragma", "no-cache")
        .header("Sec-Fetch-Dest", "document")
        .header("Sec-Fetch-Mode", "navigate")
        .header("Sec-Fetch-Site", "none")
        .header("Sec-Fetch-User", "?1")
        .header("Upgrade-Insecure-Requests", "1");
    
    // Add cookie header if present
    if !cookie_header.is_empty() {
        request = request.header("Cookie", cookie_header);
    }

    let response = request.send().await?;

    let status = response.status();
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(GscholarError::RateLimited(60));
    }

    if !status.is_success() {
        return Err(GscholarError::Api {
            code: status.as_u16() as i32,
            message: format!("HTTP error: {}", status),
        });
    }

    response
        .text()
        .await
        .map_err(|e| GscholarError::Network(e))
}

/// Parse Google Scholar HTML to extract article information.
///
/// # Arguments
///
/// * `html` - Raw HTML content from Google Scholar
///
/// # Returns
///
/// List of parsed articles
pub fn parse_result_items(html: &str) -> Result<Vec<ScholarResult>> {
    let document = Html::parse_document(html);

    // Selectors for parsing
    let item_selector =
        Selector::parse("div.gs_r.gs_or.gs_scl").map_err(|e| GscholarError::Parse(e.to_string()))?;
    let title_selector =
        Selector::parse("h3.gs_rt").map_err(|e| GscholarError::Parse(e.to_string()))?;
    let link_selector =
        Selector::parse("h3.gs_rt a").map_err(|e| GscholarError::Parse(e.to_string()))?;
    let meta_selector =
        Selector::parse("div.gs_a").map_err(|e| GscholarError::Parse(e.to_string()))?;
    let snippet_selector =
        Selector::parse("div.gs_rs").map_err(|e| GscholarError::Parse(e.to_string()))?;
    let cite_selector =
        Selector::parse("div.gs_fl.gs_flb a").map_err(|e| GscholarError::Parse(e.to_string()))?;

    let year_regex = Regex::new(r"\b(19|20)\d{2}\b").map_err(|e| GscholarError::Parse(e.to_string()))?;
    // Support both English ("Cited by X") and Chinese ("被引用 X 次") formats
    let cite_regex = Regex::new(r"(?:Cited by\s*|被引用\s*)(\d+)").map_err(|e| GscholarError::Parse(e.to_string()))?;

    let mut results = Vec::new();

    for item in document.select(&item_selector) {
        let mut data = ScholarResult::default();

        // Extract title and URL
        if let Some(title_elem) = item.select(&title_selector).next() {
            if let Some(link) = item.select(&link_selector).next() {
                data.title = link.text().collect::<String>().trim().to_string();
                data.article_url = link.value().attr("href").unwrap_or("").to_string();
            } else {
                // Title without link
                data.title = title_elem.text().collect::<String>().trim().to_string();
            }
        }

        // Extract author, year, venue from metadata
        if let Some(meta_elem) = item.select(&meta_selector).next() {
            let meta_text = meta_elem.text().collect::<String>();
            let parts: Vec<&str> = meta_text.split(" - ").collect();

            if !parts.is_empty() {
                data.author = parts[0].trim().to_string();
            }

            if parts.len() >= 2 {
                let venue_year = parts[1];
                if let Some(caps) = year_regex.captures(venue_year) {
                    if let Some(year_match) = caps.get(0) {
                        data.year = year_match.as_str().to_string();
                        let venue = venue_year[..year_match.start()].trim().trim_end_matches(',');
                        data.venue = venue.to_string();
                    }
                } else {
                    data.venue = venue_year.trim().to_string();
                }
            }
        }

        // Extract snippet
        if let Some(snippet_elem) = item.select(&snippet_selector).next() {
            data.snippet = snippet_elem.text().collect::<String>().trim().to_string();
        }

        // Extract citation count - look for "Cited by" or "被引用" links
        for link in item.select(&cite_selector) {
            let text = link.text().collect::<String>();
            // Check if this link contains citation count (href contains "cites=")
            let href = link.value().attr("href").unwrap_or("");
            if href.contains("cites=") {
                if let Some(caps) = cite_regex.captures(&text) {
                    if let Some(count) = caps.get(1) {
                        data.citations = count.as_str().to_string();
                        break;
                    }
                }
            }
        }

        // Only add if we have a title
        if !data.title.is_empty() {
            results.push(data);
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_search_url() {
        let url =
            build_search_url("https://scholar.google.com", "machine learning", 0, "0,5", Some(2020))
                .expect("Failed to build URL");
        assert!(url.as_str().contains("q=machine+learning"));
        assert!(url.as_str().contains("as_ylo=2020"));
    }

    #[test]
    fn test_parse_empty_html() {
        let results = parse_result_items("<html><body></body></html>").expect("Parse failed");
        assert!(results.is_empty());
    }
}
