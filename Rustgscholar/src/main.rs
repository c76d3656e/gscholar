//! rustgscholar - Google Scholar 3-Stage Literature Pipeline
//!
//! A Rust microservice for scraping Google Scholar, enriching with Crossref metadata,
//! and filtering by EasyScholar rankings.
//!
//! ## Usage
//!
//! ### CLI Mode
//! ```bash
//! rustgscholar search "machine learning" --pages 1-5
//! ```
//!
//! ### HTTP Server Mode
//! ```bash
//! rustgscholar serve --port 3000
//! ```

use anyhow::{Context, Result};
use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};
use chrono::Local;
use clap::{Parser, Subcommand};
use rustgscholar::{crossref::CrossrefClient, gscholar, llm_filter, openalex, rankings::RankingClient, semanticscholar, unified};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info, Level};
use tracing_subscriber::{fmt, EnvFilter};

// ============================================================================
// CLI Definition
// ============================================================================

/// Google Scholar 3-Stage Literature Pipeline - Rust Microservice
#[derive(Parser)]
#[command(name = "rustgscholar")]
#[command(version, about, long_about = None)]
struct Cli {
    /// Enable debug logging
    #[arg(short, long, global = true)]
    debug: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Search academic literature and run the pipeline
    Search {
        /// Search keywords
        keyword: String,

        /// Search source: gscholar or openalex
        #[arg(long, default_value = "gscholar", value_parser = ["gscholar", "openalex"])]
        source: String,

        /// Page range (e.g., "1", "1-10")
        #[arg(long, default_value = "1")]
        pages: String,

        /// Year filter (results from this year onwards)
        #[arg(long)]
        ylo: Option<i32>,

        /// Proxy URL (e.g., http://127.0.0.1:7890)
        #[arg(long)]
        proxy: Option<String>,

        /// Mirror site URL
        #[arg(long)]
        mirror: Option<String>,

        /// Source data type filter (default: 0,5 for articles only, excludes books)
        #[arg(long, default_value = "0,5")]
        sdt: String,

        /// Output directory
        #[arg(short, long, default_value = "./output")]
        output: PathBuf,

        // === EasyScholar Filters ===
        /// EasyScholar API key (required for filtering)
        #[arg(long)]
        easyscholar_key: Option<String>,

        /// Filter: Impact Factor >= value
        #[arg(long)]
        sciif: Option<f64>,

        /// Filter: JCI >= value
        #[arg(long)]
        jci: Option<f64>,

        /// Filter: SCI partition (e.g., "Q1")
        #[arg(long)]
        sci: Option<String>,

        /// Filter: sciUpTop (substring match)
        #[arg(long)]
        sci_up_top: Option<String>,

        /// Filter: sciBase (substring match)
        #[arg(long)]
        sci_base: Option<String>,

        /// Filter: sciUp (substring match)
        #[arg(long)]
        sci_up: Option<String>,

        // === LLM Filtering (Stage 6) ===
        /// LLM API base URL (enables Stage 6, e.g., https://api.openai.com/v1)
        #[arg(long)]
        llm_base_url: Option<String>,

        /// LLM API key
        #[arg(long)]
        llm_key: Option<String>,

        /// LLM model name
        #[arg(long, default_value = "gpt-4o-mini")]
        llm_model: String,

        /// Filter keywords/phrases for LLM guidance (e.g., "landslide,slope,边坡")
        #[arg(long)]
        filter_help: Option<String>,
    },

    /// Run as HTTP server
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },

    /// Manage cookies
    Cookies {
        #[command(subcommand)]
        action: CookieAction,
    },
}

#[derive(Subcommand)]
enum CookieAction {
    /// Clear stored cookies
    Clear,
    /// Show cookie file path
    Path,
    /// Fetch cookies from browser (opens Google Scholar)
    Fetch,
}

// ============================================================================
// Main Entry Point
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let log_level = if cli.debug { Level::DEBUG } else { Level::INFO };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(log_level.to_string()));

    fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_thread_ids(false)
        .init();

    match cli.command {
        Commands::Search {
            keyword,
            source,
            pages,
            ylo,
            proxy,
            mirror,
            sdt,
            output,
            easyscholar_key,
            sciif,
            jci,
            sci,
            sci_up_top,
            sci_base,
            sci_up,
            llm_base_url,
            llm_key,
            llm_model,
            filter_help,
        } => {
            run_search_pipeline(
                keyword,
                source,
                pages,
                ylo,
                proxy,
                mirror,
                sdt,
                output,
                easyscholar_key,
                sciif,
                jci,
                sci,
                sci_up_top,
                sci_base,
                sci_up,
                llm_base_url,
                llm_key,
                llm_model,
                filter_help,
            )
            .await
        }
        Commands::Serve { port, host } => run_server(host, port).await,
        Commands::Cookies { action } => handle_cookies(action),
    }
}

// ============================================================================
// Search Pipeline
// ============================================================================

#[allow(clippy::too_many_arguments)]
async fn run_search_pipeline(
    keyword: String,
    source: String,
    pages_str: String,
    ylo: Option<i32>,
    proxy: Option<String>,
    mirror: Option<String>,
    sdt: String,
    output_dir: PathBuf,
    easyscholar_key: Option<String>,
    sciif: Option<f64>,
    jci: Option<f64>,
    sci: Option<String>,
    sci_up_top: Option<String>,
    sci_base: Option<String>,
    sci_up: Option<String>,
    llm_base_url: Option<String>,
    llm_key: Option<String>,
    llm_model: String,
    filter_help: Option<String>,
) -> Result<()> {
    // Parse pages
    let pages = parse_pages(&pages_str).context("Invalid --pages format")?;

    // Calculate year filter (default: current year - 5)
    let ylo_val = ylo.unwrap_or_else(|| Local::now().format("%Y").to_string().parse().unwrap_or(2020) - 5);

    // Create output folder
    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
    let safe_keyword: String = keyword
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
        .collect::<String>()
        .trim()
        .replace(' ', "_");
    let output_folder = output_dir.join(format!("{}_{}", timestamp, safe_keyword));
    std::fs::create_dir_all(&output_folder).context("Failed to create output directory")?;

    println!("Output folder: {}", output_folder.display());

    // ===========================================
    // STAGE 1: Google Scholar Scrape
    // ===========================================
    // ===========================================
    // STAGE 1 & 2: Search & Enrichment
    // ===========================================
    
    let mut enriched_list: Vec<EnrichedResult> = Vec::new();

    if source == "bs" || source == "gscholar" {
        println!("\n--- Stage 1: Google Scholar Search ---");

        let query_options = gscholar::QueryOptions {
            proxy: proxy.clone(),
            pages: pages.clone(),
            sdt,
            ylo: Some(ylo_val),
            base_url: mirror,
            all_results: true,
        };

        let gs_results = gscholar::query(&keyword, &query_options).await?;

        if gs_results.is_empty() {
            println!("No results from Google Scholar.");
            return Ok(());
        }

        println!("Found {} results from Google Scholar.", gs_results.len());

        // Save Stage 1 CSV
        let gs_path = output_folder.join("1_gscholar.csv");
        save_csv(&gs_path, &gs_results, &["title", "author", "year", "venue", "article_url", "citations", "snippet"])?;

        // ===========================================
        // STAGE 2: Crossref Enrichment
        // ===========================================
        println!("\n--- Stage 2: Crossref Enrichment ---");

        let crossref_client = CrossrefClient::new(3)?;
        let titles: Vec<String> = gs_results.iter().map(|r| r.title.clone()).collect();

        println!("Looking up {} titles (concurrent, 3 workers)...", titles.len());
        let crossref_results: Vec<Option<rustgscholar::crossref::CrossrefMetadata>> = crossref_client.lookup_batch(&titles).await;

        // Merge results
        enriched_list = Vec::with_capacity(gs_results.len());
        for (gs, cr) in gs_results.iter().zip(crossref_results.iter()) {
            let enriched = EnrichedResult {
                title: gs.title.clone(),
                author: gs.author.clone(),
                year: gs.year.clone(),
                publication_date: cr.as_ref().map(|c| c.date.clone()).unwrap_or_default(), // Use crossref date
                venue: gs.venue.clone(),
                article_url: gs.article_url.clone(),
                citations: gs.citations.clone(),
                snippet: gs.snippet.clone(),
                doi: cr.as_ref().map(|c| c.doi.clone()).unwrap_or_default(),
                journal: cr.as_ref().map(|c| c.journal.clone()).unwrap_or_default(),
                crossref_authors: cr.as_ref().map(|c| c.authors.clone()).unwrap_or_default(),
                crossref_date: cr.as_ref().map(|c| c.date.clone()).unwrap_or_default(),
                abstract_text: cr.as_ref().map(|c| c.abstract_text.clone()).unwrap_or_default(),
                // Rankings (to be filled in Stage 3)
                if_score: String::new(),
                jci_score: String::new(),
                sci_partition: String::new(),
                sci_up_top: String::new(),
                sci_base: String::new(),
                sci_up: String::new(),
            };
            enriched_list.push(enriched);
        }

        let matched = crossref_results.iter().filter(|r| r.is_some()).count();
        println!("Crossref: {} / {} matched", matched, titles.len());

        // Save Stage 2 CSV
        let cr_path = output_folder.join("2_crossref.csv");
        save_csv(&cr_path, &enriched_list, &["title", "doi", "journal", "author", "crossref_authors", "crossref_date", "abstract_text", "article_url", "citations"])?;

    } else if source == "openalex" {
        println!("\n--- Stage 1: OpenAlex Search (Enriched) ---");

        let query_options = openalex::QueryOptions {
            pages: pages.clone(),
            ylo: Some(ylo_val),
            yhi: None,
            all_results: true,
        };

        let oa_results = openalex::query(&keyword, &query_options).await?;

        if oa_results.is_empty() {
            println!("No results from OpenAlex.");
            return Ok(());
        }

        println!("Found {} results from OpenAlex.", oa_results.len());

        // Save Stage 1 CSV with all OpenAlex fields
        let oa_path = output_folder.join("1_openalex.csv");
        save_csv(&oa_path, &oa_results, &[
            "title", "author", "year", "publication_date", "venue", "source_type", "doi",
            "article_url", "pdf_url", "citations", "is_oa", "oa_status", "oa_url",
            "language", "work_type", "keywords", "primary_topic",
            "referenced_works", "related_works",
            "referenced_works_count", "related_works_count", "locations_count",
            "snippet", "openalex_id"
        ])?;

        // Convert to EnrichedResult for Stage 3
        enriched_list = oa_results.into_iter().map(|oa| {
            EnrichedResult {
                title: oa.title,
                author: oa.author,
                year: oa.year,
                publication_date: oa.publication_date, // ISO date from OpenAlex
                venue: oa.venue.clone(),
                article_url: oa.article_url,
                citations: oa.citations,
                snippet: oa.snippet.clone(),
                doi: oa.doi,
                journal: oa.venue, // Map venue to journal for ranking lookup
                crossref_authors: String::new(),
                crossref_date: String::new(),
                abstract_text: oa.snippet, // Use snippet as abstract
                if_score: String::new(),
                jci_score: String::new(),
                sci_partition: String::new(),
                sci_up_top: String::new(),
                sci_base: String::new(),
                sci_up: String::new(),
            }
        }).collect();

    } else {
        anyhow::bail!("Invalid source: {}", source);
    }

    // ===========================================
    // STAGE 3: EasyScholar Ranking Enrichment
    // ===========================================
    if let Some(key) = easyscholar_key {
        println!("\n--- Stage 3: EasyScholar Ranking ---");

        let ranking_client = RankingClient::new(key)?;

        let filter_active = sciif.is_some()
            || jci.is_some()
            || sci.is_some()
            || sci_up_top.is_some()
            || sci_base.is_some()
            || sci_up.is_some();

        // Step 1: Collect unique journal names
        let unique_journals: std::collections::HashSet<String> = enriched_list
            .iter()
            .map(|item| item.journal.trim().to_string())
            .filter(|j| !j.is_empty())
            .collect();

        println!("Found {} unique journals to query", unique_journals.len());

        // Step 2: Batch query all unique journals
        use std::collections::HashMap;
        let mut journal_rankings: HashMap<String, Option<rustgscholar::rankings::RankingMetrics>> = HashMap::new();
        
        for (idx, journal) in unique_journals.iter().enumerate() {
            if (idx + 1) % 50 == 0 {
                println!("  Queried {}/{} journals...", idx + 1, unique_journals.len());
            }
            let metrics = ranking_client.get_rank(journal).await;
            journal_rankings.insert(journal.clone(), metrics);
        }

        println!("Completed querying {} journals", unique_journals.len());

        // Step 3: Assign rankings to all articles
        let mut result_list: Vec<EnrichedResult> = Vec::new();

        for mut item in enriched_list {
            let journal = item.journal.trim().to_string();
            
            if journal.is_empty() {
                if !filter_active {
                    result_list.push(item);
                }
                continue;
            }

            let metrics = match journal_rankings.get(&journal).cloned().flatten() {
                Some(m) => m,
                None => {
                    if !filter_active {
                        result_list.push(item);
                    }
                    continue;
                }
            };

            // Check filters
            let mut keep = true;

            if filter_active {
                if let Some(threshold) = sciif {
                    if !RankingClient::passes_numeric_filter(metrics.sciif.as_deref(), threshold) {
                        keep = false;
                    }
                }

                if keep {
                    if let Some(threshold) = jci {
                        if !RankingClient::passes_numeric_filter(metrics.jci.as_deref(), threshold) {
                            keep = false;
                        }
                    }
                }

                if keep {
                    if let Some(ref pattern) = sci {
                        if !RankingClient::passes_string_filter(metrics.sci.as_deref(), pattern) {
                            keep = false;
                        }
                    }
                }

                if keep {
                    if let Some(ref pattern) = sci_up_top {
                        if !RankingClient::passes_string_filter(metrics.sci_up_top.as_deref(), pattern) {
                            keep = false;
                        }
                    }
                }

                if keep {
                    if let Some(ref pattern) = sci_base {
                        if !RankingClient::passes_string_filter(metrics.sci_base.as_deref(), pattern) {
                            keep = false;
                        }
                    }
                }

                if keep {
                    if let Some(ref pattern) = sci_up {
                        if !RankingClient::passes_string_filter(metrics.sci_up.as_deref(), pattern) {
                            keep = false;
                        }
                    }
                }
            }

            if keep {
                item.if_score = metrics.sciif.unwrap_or_default();
                item.jci_score = metrics.jci.unwrap_or_default();
                item.sci_partition = metrics.sci.unwrap_or_default();
                item.sci_up_top = metrics.sci_up_top.unwrap_or_default();
                item.sci_base = metrics.sci_base.unwrap_or_default();
                item.sci_up = metrics.sci_up.unwrap_or_default();
                result_list.push(item);
            }
        }

        if filter_active {
            println!("Filtered: {} results", result_list.len());
        } else {
            println!("Enriched: {} results with ranking data", result_list.len());
        }

        // Save Stage 3 CSV
        let es_path = output_folder.join("3_easyscholar.csv");
        save_csv(&es_path, &result_list, &["title", "if_score", "jci_score", "sci_partition", "journal", "doi", "author", "abstract_text", "article_url"])?;

        // ===========================================
        // STAGE 4: Semantic Scholar Enrichment
        // ===========================================
        if !result_list.is_empty() {
            println!("\n--- Stage 4: Semantic Scholar Lookup ---");

            // Extract DOIs from result_list
            let dois: Vec<String> = result_list
                .iter()
                .map(|r| r.doi.clone())
                .filter(|d: &String| !d.is_empty())
                .collect();

            if dois.is_empty() {
                println!("No DOIs found in filtered results, skipping Semantic Scholar.");
            } else {
                println!("Looking up {} papers by DOI...", dois.len());

                // Batch lookup (no API key for now - can be added later)
                match semanticscholar::batch_lookup(&dois, None).await {
                    Ok(ss_results) => {
                        println!("Found {} papers in Semantic Scholar.", ss_results.len());

                        // Save Stage 4 CSV with DOI as key for cross-filtering
                        let ss_path = output_folder.join("4_semanticscholar.csv");
                        save_csv(&ss_path, &ss_results, &[
                            "doi", "title", "ss_abstract", "tldr", "ss_url", "is_oa", "oa_pdf_url", "paper_id", "embedding"
                        ])?;

                        // ===========================================
                        // STAGE 5: Unified CSV Generation
                        // ===========================================
                        println!("\n--- Stage 5: Creating Unified Dataset ---");

                        // Convert result_list to EnrichedInput for unified module
                        let enriched_inputs: Vec<unified::EnrichedInput> = result_list.iter()
                            .map(|r| unified::EnrichedInput {
                                title: r.title.clone(),
                                author: r.author.clone(),
                                year: r.year.clone(),
                                publication_date: r.publication_date.clone(),
                                doi: r.doi.clone(),
                                article_url: r.article_url.clone(),
                                abstract_text: r.abstract_text.clone(),
                                journal: r.journal.clone(),
                                if_score: r.if_score.clone(),
                                jci_score: r.jci_score.clone(),
                                sci_partition: r.sci_partition.clone(),
                            })
                            .collect();

                        // Generate unified results using the module
                        let unified_results = unified::generate_unified(&enriched_inputs, &ss_results);

                        // Save Stage 5 CSV
                        let unified_path = output_folder.join("5_unified.csv");
                        save_csv(&unified_path, &unified_results, unified::UNIFIED_COLUMNS)?;
                        println!("Created unified dataset: {} papers", unified_results.len());

                        // ===========================================
                        // STAGE 6: LLM Relevance Filtering
                        // ===========================================
                        if let Some(ref base_url) = llm_base_url {
                            if let Some(ref api_key) = llm_key {
                                println!("\n--- Stage 6: LLM Relevance Filtering ---");
                                
                                let llm_config = llm_filter::LlmConfig {
                                    base_url: base_url.clone(),
                                    api_key: api_key.clone(),
                                    model: llm_model.clone(),
                                    filter_help: filter_help.clone().unwrap_or_default(),
                                };

                                println!(
                                    "Filtering {} papers with {} (max 10 concurrent requests)...",
                                    unified_results.len(),
                                    llm_config.model
                                );

                                match llm_filter::filter_papers(&llm_config, &unified_results).await {
                                    Ok((filter_results, usage)) => {
                                        // Save filtered results
                                        let filtered_path = output_folder.join("6_llm_filtered.csv");
                                        save_csv(&filtered_path, &filter_results, &[
                                            "id", "title", "label", "confidence", "evidence", "reason"
                                        ])?;

                                        // Count by label
                                        let relevant = filter_results.iter().filter(|r| r.label == "relevant").count();
                                        let irrelevant = filter_results.iter().filter(|r| r.label == "irrelevant").count();
                                        let uncertain = filter_results.iter().filter(|r| r.label == "uncertain").count();

                                        println!(
                                            "LLM filtering complete: {} relevant, {} irrelevant, {} uncertain",
                                            relevant, irrelevant, uncertain
                                        );

                                        // Log token usage
                                        let usage_path = output_folder.join("6_token_usage.log");
                                        let usage_line = format!(
                                            "{},{},{},{}",
                                            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                                            usage.prompt_tokens,
                                            usage.completion_tokens,
                                            usage.total_tokens
                                        );
                                        std::fs::write(&usage_path, &usage_line)
                                            .context("Failed to write token usage log")?;
                                        println!(
                                            "Token usage: {} prompt + {} completion = {} total",
                                            usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
                                        );

                                        // ===========================================
                                        // STAGE 7: Relevant Papers Only
                                        // ===========================================
                                        println!("\n--- Stage 7: Extracting Relevant Papers ---");

                                        // Get DOIs of relevant papers
                                        let relevant_dois: std::collections::HashSet<String> = filter_results
                                            .iter()
                                            .filter(|r| r.label == "relevant")
                                            .map(|r| r.id.to_lowercase())
                                            .collect();

                                        // Filter unified_results to only keep relevant papers
                                        let relevant_papers: Vec<&unified::UnifiedResult> = unified_results
                                            .iter()
                                            .filter(|u| relevant_dois.contains(&u.doi.to_lowercase()))
                                            .collect();

                                        if !relevant_papers.is_empty() {
                                            // Create a new struct for CSV output with full data
                                            #[derive(serde::Serialize)]
                                            struct RelevantPaper {
                                                title: String,
                                                author: String,
                                                date: String,
                                                doi: String,
                                                article_url: String,
                                                pdf_url: String,
                                                abstract_text: String,
                                                tldr: String,
                                                journal: String,
                                                if_score: String,
                                                jci_score: String,
                                                sci_partition: String,
                                                confidence: f64,
                                                evidence: String,
                                                reason: String,
                                            }

                                            // Join filter_results with unified_results
                                            let filter_map: std::collections::HashMap<String, &llm_filter::FilterResult> = 
                                                filter_results.iter()
                                                    .filter(|r| r.label == "relevant")
                                                    .map(|r| (r.id.to_lowercase(), r))
                                                    .collect();

                                            let relevant_output: Vec<RelevantPaper> = relevant_papers
                                                .iter()
                                                .filter_map(|u| {
                                                    filter_map.get(&u.doi.to_lowercase()).map(|f| RelevantPaper {
                                                        title: u.title.clone(),
                                                        author: u.author.clone(),
                                                        date: u.date.clone(),
                                                        doi: u.doi.clone(),
                                                        article_url: u.article_url.clone(),
                                                        pdf_url: u.pdf_url.clone(),
                                                        abstract_text: u.abstract_text.clone(),
                                                        tldr: u.tldr.clone(),
                                                        journal: u.journal.clone(),
                                                        if_score: u.if_score.clone(),
                                                        jci_score: u.jci_score.clone(),
                                                        sci_partition: u.sci_partition.clone(),
                                                        confidence: f.confidence,
                                                        evidence: f.evidence.clone(),
                                                        reason: f.reason.clone(),
                                                    })
                                                })
                                                .collect();

                                            let relevant_path = output_folder.join("7_relevant.csv");
                                            save_csv(&relevant_path, &relevant_output, &[
                                                "title", "author", "date", "doi", "article_url", "pdf_url",
                                                "abstract_text", "tldr", "journal", "if_score", "jci_score", 
                                                "sci_partition", "confidence", "evidence", "reason"
                                            ])?;
                                            println!("Saved {} relevant papers to 7_relevant.csv", relevant_output.len());
                                        } else {
                                            println!("No relevant papers found.");
                                        }
                                    }
                                    Err(e) => {
                                        println!("LLM filtering failed: {}", e);
                                    }
                                }
                            } else {
                                println!("\n--- Stage 6: Skipped (--llm-key not provided) ---");
                            }
                        } else {
                            println!("\n--- Stage 6: Skipped (no --llm-base-url provided) ---");
                        }
                    }
                    Err(e) => {
                        println!("Semantic Scholar lookup failed: {}", e);
                    }
                }
            }
        }
    } else {
        println!("\n--- Stage 3: Skipped (no --easyscholar-key provided) ---");
    }

    println!("\n✓ Pipeline complete. Results in: {}", output_folder.display());
    Ok(())
}

/// Parse page range string (e.g., "1", "1-10")
fn parse_pages(pages_str: &str) -> Result<Vec<i32>> {
    if pages_str.contains('-') {
        let parts: Vec<&str> = pages_str.split('-').collect();
        if parts.len() != 2 {
            anyhow::bail!("Invalid page range format");
        }
        let start: i32 = parts[0].parse().context("Invalid start page")?;
        let end: i32 = parts[1].parse().context("Invalid end page")?;
        Ok((start..=end).collect())
    } else {
        let page: i32 = pages_str.parse().context("Invalid page number")?;
        Ok(vec![page])
    }
}

/// Enriched result combining Google Scholar and Crossref data
#[derive(Debug, Serialize, Deserialize)]
struct EnrichedResult {
    title: String,
    author: String,
    year: String,
    publication_date: String,  // ISO date (YYYY-MM-DD) from OpenAlex
    venue: String,
    article_url: String,
    citations: String,
    snippet: String,
    doi: String,
    journal: String,
    crossref_authors: String,
    crossref_date: String,
    abstract_text: String,
    if_score: String,
    jci_score: String,
    sci_partition: String,
    sci_up_top: String,
    sci_base: String,
    sci_up: String,
}

/// Save data to CSV file
fn save_csv<T: Serialize>(path: &std::path::Path, data: &[T], _priority_fields: &[&str]) -> Result<()> {
    if data.is_empty() {
        println!("No data to save to {:?}", path);
        return Ok(());
    }

    let mut wtr = csv::WriterBuilder::new()
        .has_headers(true)
        .from_path(path)
        .context("Failed to create CSV writer")?;

    for item in data {
        wtr.serialize(item).context("Failed to write CSV record")?;
    }

    wtr.flush().context("Failed to flush CSV")?;
    println!("Saved: {:?}", path);
    Ok(())
}

// ============================================================================
// HTTP Server
// ============================================================================

async fn run_server(host: String, port: u16) -> Result<()> {
    info!(host = %host, port = port, "Starting HTTP server");
    println!("Starting server at http://{}:{}", host, port);

    // Shared state (could add database connections, etc.)
    let app_state = Arc::new(AppState::default());

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/search", post(search_handler))
        .with_state(app_state);

    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .context("Invalid host:port")?;

    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("Listening on http://{}", addr);

    axum::serve(listener, app)
        .await
        .context("Server error")?;

    Ok(())
}

#[derive(Default)]
struct AppState {
    // Add shared state here (e.g., rate limiters, caches)
}

/// Health check endpoint
async fn health_handler() -> &'static str {
    "OK"
}

/// Search request body
#[derive(Debug, Deserialize)]
struct SearchRequest {
    keyword: String,
    #[serde(default = "default_pages")]
    pages: Vec<i32>,
    ylo: Option<i32>,
    proxy: Option<String>,
}

fn default_pages() -> Vec<i32> {
    vec![1]
}

/// Search response
#[derive(Debug, Serialize)]
struct SearchResponse {
    status: String,
    count: usize,
    results: Vec<gscholar::ScholarResult>,
}

/// Search endpoint handler
async fn search_handler(
    State(_state): State<Arc<AppState>>,
    Json(req): Json<SearchRequest>,
) -> Json<SearchResponse> {
    info!(keyword = %req.keyword, pages = ?req.pages, "Search request");

    let options = gscholar::QueryOptions {
        proxy: req.proxy,
        pages: req.pages,
        ylo: req.ylo,
        ..Default::default()
    };

    match gscholar::query(&req.keyword, &options).await {
        Ok(results) => Json(SearchResponse {
            status: "success".to_string(),
            count: results.len(),
            results,
        }),
        Err(e) => {
            error!(error = %e, "Search failed");
            Json(SearchResponse {
                status: format!("error: {}", e),
                count: 0,
                results: vec![],
            })
        }
    }
}

// ============================================================================
// Cookie Management
// ============================================================================

fn handle_cookies(action: CookieAction) -> Result<()> {
    use rustgscholar::cookies::CookieManager;

    let manager = CookieManager::new()?;

    match action {
        CookieAction::Clear => {
            manager.clear()?;
            println!("Cookies cleared.");
        }
        CookieAction::Path => {
            println!("Cookie file: {:?}", manager.path());
        }
        CookieAction::Fetch => {
            println!("Opening Google Scholar in browser to fetch cookies...");
            println!("Please complete any CAPTCHA or login, then the cookies will be saved.");
            println!();
            println!("Cookie file will be saved to: {:?}", manager.path());
            println!();
            
            // Run async cookie fetch
            tokio::runtime::Runtime::new()?
                .block_on(fetch_cookies_from_browser(&manager))?;
        }
    }

    Ok(())
}

/// Fetch cookies from browser using headless-chrome or browser subagent
async fn fetch_cookies_from_browser(manager: &rustgscholar::cookies::CookieManager) -> Result<()> {
    use std::io::{self, Write};
    
    println!("=== Manual Cookie Export Instructions ===");
    println!();
    println!("Since automated browser cookie fetching requires additional setup,");
    println!("please follow these steps to export cookies manually:");
    println!();
    println!("1. Open Google Chrome and go to: https://scholar.google.com");
    println!("2. Complete any CAPTCHA if prompted");
    println!("3. Press F12 to open Developer Tools");
    println!("4. Go to 'Application' tab -> 'Cookies' -> 'https://scholar.google.com'");
    println!("5. Right-click and copy all cookies, or use a cookie export extension");
    println!();
    println!("Alternatively, paste cookies in JSON format below (or press Enter to skip):");
    println!("Format: [{{\"name\":\"NID\",\"value\":\"xxx\",\"domain\":\".google.com\"}},...]");
    println!();
    print!("> ");
    io::stdout().flush()?;
    
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();
    
    if input.is_empty() {
        println!("No cookies provided. You can manually create the cookie file at:");
        println!("{:?}", manager.path());
        return Ok(());
    }
    
    // Try to parse as JSON
    match serde_json::from_str::<Vec<rustgscholar::cookies::Cookie>>(input) {
        Ok(cookies) => {
            manager.save(&cookies)?;
            println!("Successfully saved {} cookies!", cookies.len());
        }
        Err(e) => {
            println!("Failed to parse cookies: {}", e);
            println!("Please ensure the format is valid JSON.");
        }
    }
    
    Ok(())
}
