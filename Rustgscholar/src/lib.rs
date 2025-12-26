//! # rustgscholar
//!
//! Google Scholar 3-Stage Literature Pipeline - Rust Microservice
//!
//! ## Modules
//!
//! - [`gscholar`] - Google Scholar scraping with Playwright
//! - [`crossref`] - Crossref API client for metadata enrichment
//! - [`rankings`] - EasyScholar rankings API
//! - [`cookies`] - Cookie persistence
//! - [`error`] - Custom error types
//!
//! ## Usage
//!
//! ```rust,no_run
//! use rustgscholar::{gscholar, crossref, rankings};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let results = gscholar::query("machine learning", &Default::default()).await?;
//!     println!("Found {} results", results.len());
//!     Ok(())
//! }
//! ```

pub mod cookies;
pub mod crossref;
pub mod error;
pub mod gscholar;
pub mod llm_filter;
pub mod openalex;
pub mod prompts;
pub mod rankings;
pub mod semanticscholar;
pub mod unified;

pub use error::{GscholarError, Result};
