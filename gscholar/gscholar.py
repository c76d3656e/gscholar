"""
Google Scholar scraper using Playwright.

This module provides the core scraping functionality for Google Scholar
using Playwright for browser automation with anti-detection features.
"""

import asyncio
import json
import logging
import os
import random
import re
import subprocess
from html.entities import name2codepoint
from urllib.parse import urlencode

from bs4 import BeautifulSoup
from playwright.async_api import async_playwright

from gscholar.cookies import COOKIE_FILE

# Output format constants (kept for backward compatibility)
FORMAT_BIBTEX = 4
FORMAT_ENDNOTE = 3
FORMAT_REFMAN = 2
FORMAT_WENXIANWANG = 5

DEFAULT_SCHOLAR_URL = "https://scholar.google.com"

logger = logging.getLogger(__name__)


def query(
    searchstr: str,
    outformat: int = FORMAT_BIBTEX,
    allresults: bool = False,
    proxy: str | None = None,
    pages: list[int] | None = None,
    sdt: str = "0,5",
    ylo: int | None = None,
    base_url: str | None = None,
) -> list[dict]:
    """Query Google Scholar and return results.
    
    Uses Playwright with concurrent page fetching for speed.
    Synchronous wrapper around async implementation.
    
    Parameters
    ----------
    searchstr : str
        Search query string.
    outformat : int
        Output format constant (kept for API compatibility).
    allresults : bool
        If True, return all results. If False, return first per page.
    proxy : str, optional
        Proxy server URL (e.g., "http://127.0.0.1:7890").
    pages : list[int], optional
        List of page numbers to fetch (default: [1]).
    sdt : str
        Source data type filter (default: "0,5" for articles only).
    ylo : int, optional
        Year low filter (results from this year onwards).
    base_url : str, optional
        Custom base URL for mirror sites.
    
    Returns
    -------
    list[dict]
        List of result dicts with keys: title, author, year, venue,
        article_url, citations, snippet.
    """
    return asyncio.run(_query_async(
        searchstr=searchstr,
        outformat=outformat,
        allresults=allresults,
        proxy=proxy,
        pages=pages,
        sdt=sdt,
        ylo=ylo,
        base_url=base_url
    ))


async def _query_async(
    searchstr: str,
    outformat: int,
    allresults: bool,
    proxy: str | None,
    pages: list[int] | None,
    sdt: str,
    ylo: int | None,
    base_url: str | None,
) -> list[dict]:
    """Async implementation of Google Scholar query."""
    scholar_url = base_url.rstrip("/") if base_url else DEFAULT_SCHOLAR_URL
    logger.debug(f"Query: {searchstr} on {scholar_url}")
    
    if pages is None:
        pages = [1]
    
    full_results: list[dict] = []
    
    async with async_playwright() as p:
        # Launch headless browser with anti-detection
        browser = await p.chromium.launch(
            headless=True,
            args=["--disable-blink-features=AutomationControlled"]
        )
        
        # Configure browser context
        context_options = {
            "viewport": {"width": 1920, "height": 1080},
            "locale": "en-US",
            "timezone_id": "America/New_York",
            "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
        }
        if proxy:
            context_options["proxy"] = {"server": proxy}
        
        context = await browser.new_context(**context_options)
        
        # Load cookies if available
        if COOKIE_FILE.exists():
            try:
                with open(COOKIE_FILE, "r", encoding="utf-8") as f:
                    cookies = json.load(f)
                    if isinstance(cookies, list):
                        await context.add_cookies(cookies)
            except Exception as e:
                logger.warning(f"Failed to load cookies: {e}")
        
        # Anti-detection script
        await context.add_init_script(
            "Object.defineProperty(navigator, 'webdriver', {get: () => undefined})"
        )
        
        # Concurrency control (max 3 pages at once)
        sem = asyncio.Semaphore(3)
        
        async def fetch_page(page_num: int) -> list[dict]:
            """Fetch a single search results page."""
            async with sem:
                start = (page_num - 1) * 10
                if start < 0:
                    start = 0
                
                params = {"q": searchstr, "start": start, "as_sdt": sdt}
                if ylo is not None:
                    params["as_ylo"] = ylo
                
                url = f"{scholar_url}/scholar?{urlencode(params)}"
                page = await context.new_page()
                
                try:
                    # Random delay to avoid detection
                    await asyncio.sleep(random.uniform(0.5, 2.0))
                    
                    logger.info(f"Fetching page {page_num}...")
                    response = await page.goto(url, wait_until="domcontentloaded")
                    content = await page.content()
                    
                    # Check for CAPTCHA or rate limit
                    if "Solving the above CAPTCHA" in content or "robot" in await page.title():
                        logger.warning(f"CAPTCHA detected on page {page_num}")
                        return []
                    
                    if response and response.status == 429:
                        logger.error(f"Rate limited (429) on page {page_num}")
                        return []
                    
                    return parse_result_items(content)
                    
                except Exception as e:
                    logger.error(f"Error fetching page {page_num}: {e}")
                    return []
                finally:
                    await page.close()
        
        # Fetch all pages concurrently
        tasks = [fetch_page(p) for p in pages]
        results_list = await asyncio.gather(*tasks)
        
        # Flatten results
        for r in results_list:
            if allresults:
                full_results.extend(r)
            elif r:
                full_results.append(r[0])
        
        await browser.close()
    
    return full_results


def parse_result_items(html: str) -> list[dict]:
    """Parse Google Scholar HTML to extract article information.
    
    Parameters
    ----------
    html : str
        Raw HTML content from Google Scholar.
    
    Returns
    -------
    list[dict]
        Parsed articles with title, author, year, venue, etc.
    """
    soup = BeautifulSoup(html, "html.parser")
    results = []
    
    for item in soup.select("div.gs_r.gs_or.gs_scl"):
        data = {
            "title": "",
            "author": "",
            "year": "",
            "venue": "",
            "article_url": "",
            "citations": "0",
            "snippet": "",
        }
        
        # Extract title and URL
        title_elem = item.select_one("h3.gs_rt")
        if title_elem:
            for span in title_elem.select("span"):
                span.decompose()
            title_link = title_elem.select_one("a")
            if title_link:
                data["title"] = title_link.get_text(strip=True)
                data["article_url"] = title_link.get("href", "")
            else:
                data["title"] = title_elem.get_text(strip=True)
        
        # Extract author, year, venue from metadata
        meta_elem = item.select_one("div.gs_a")
        if meta_elem:
            meta_text = meta_elem.get_text(strip=True)
            parts = meta_text.split(" - ")
            if len(parts) >= 1:
                data["author"] = parts[0].strip()
            if len(parts) >= 2:
                venue_year = parts[1]
                year_match = re.search(r'\b(19|20)\d{2}\b', venue_year)
                if year_match:
                    data["year"] = year_match.group(0)
                    data["venue"] = venue_year[:year_match.start()].strip().rstrip(',')
                else:
                    data["venue"] = venue_year.strip()
        
        # Extract snippet
        snippet_elem = item.select_one("div.gs_rs")
        if snippet_elem:
            data["snippet"] = snippet_elem.get_text(strip=True)
        
        # Extract citation count
        for link in item.select("div.gs_fl.gs_flb a"):
            text = link.get_text(strip=True)
            if "Cited by" in text:
                match = re.search(r"Cited by (\d+)", text)
                if match:
                    data["citations"] = match.group(1)
                break
        
        if data["title"]:
            results.append(data)
    
    return results


# =============================================================================
# Legacy utility functions (kept for backward compatibility)
# =============================================================================

def convert_pdf_to_txt(pdf: str, startpage: int | None = None) -> str:
    """Convert PDF to text using pdftotext command line tool."""
    startpageargs = ["-f", str(startpage)] if startpage else []
    try:
        stdout = subprocess.Popen(
            ["pdftotext", "-q"] + startpageargs + [pdf, "-"],
            stdout=subprocess.PIPE,
        ).communicate()[0]
        return stdout.decode()
    except Exception as e:
        logger.error(f"Error converting PDF: {e}")
        return ""


def pdflookup(
    pdf: str,
    allresults: bool,
    outformat: int,
    startpage: int | None = None,
    proxy: str | None = None,
) -> list[dict]:
    """Look up a PDF on Google Scholar by extracting text and searching."""
    txt = convert_pdf_to_txt(pdf, startpage)
    txt = re.sub(r"\W", " ", txt)
    words = txt.strip().split()[:20]
    gsquery = " ".join(words)
    return query(gsquery, outformat, allresults, proxy=proxy)


def rename_file(pdf: str, bibitem: str) -> None:
    """Rename PDF file based on bibliographic information."""
    def get_element(element: str) -> str | None:
        for line in bibitem.split("\n"):
            line = line.strip()
            if line.startswith(element):
                value = line.split("=", 1)[-1].strip()
                while value.endswith(","):
                    value = value[:-1]
                while value.startswith("{") or value.startswith('"'):
                    value = value[1:-1]
                return value
        return None
    
    year = get_element("year")
    author = get_element("author")
    if author:
        author = author.split(",")[0]
    title = get_element("title")
    
    parts = [p for p in (year, author, title) if p]
    filename = "-".join(parts) + ".pdf"
    newfile = pdf.replace(os.path.basename(pdf), filename)
    logger.info(f"Renaming {pdf} to {newfile}")
    os.rename(pdf, newfile)
