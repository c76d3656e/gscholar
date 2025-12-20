"""Cookie management for Google Scholar requests.

This module uses Playwright to harvest cookies from a real browser session,
making subsequent requests appear more legitimate.
"""

import json
import logging
import os
from pathlib import Path
from typing import Any

logger = logging.getLogger(__name__)

# Default cookie storage path
COOKIE_FILE = Path.home() / ".gscholar_cookies.json"


def get_cookie_path() -> Path:
    """Get the path to the cookie file."""
    return COOKIE_FILE


def load_cookies() -> dict[str, str]:
    """Load cookies from the file.
    
    Returns
    -------
    dict[str, str]
        Dictionary of cookie name -> value pairs.
    """
    if not COOKIE_FILE.exists():
        return {}
    
    try:
        with open(COOKIE_FILE, "r", encoding="utf-8") as f:
            data = json.load(f)
            # Convert list format to dict format if needed
            if isinstance(data, list):
                return {c["name"]: c["value"] for c in data}
            return data
    except (json.JSONDecodeError, KeyError) as e:
        logger.warning(f"Failed to load cookies: {e}")
        return {}


def save_cookies(cookies: list[dict[str, Any]]) -> None:
    """Save cookies to the file.
    
    Parameters
    ----------
    cookies
        List of cookie dictionaries from Playwright.
    """
    try:
        with open(COOKIE_FILE, "w", encoding="utf-8") as f:
            json.dump(cookies, f, indent=2)
        logger.info(f"Cookies saved to {COOKIE_FILE}")
    except OSError as e:
        logger.error(f"Failed to save cookies: {e}")


def harvest_cookies(url: str = "https://scholar.google.com") -> dict[str, str]:
    """Harvest cookies by visiting the site with a real browser.
    
    This function launches a headless browser, visits the target site,
    and extracts all cookies for future use.
    
    Parameters
    ----------
    url
        The URL to visit for cookie harvesting. Default is Google Scholar.
        
    Returns
    -------
    dict[str, str]
        Dictionary of cookie name -> value pairs.
    """
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        logger.error("Playwright not installed. Run: pip install playwright && playwright install chromium")
        return {}
    
    logger.info(f"Harvesting cookies from {url}...")
    print(f"Launching browser to harvest cookies from {url}...")
    
    cookies_dict: dict[str, str] = {}
    
    try:
        with sync_playwright() as p:
            # Launch browser with typical user settings
            browser = p.chromium.launch(
                headless=True,
                args=[
                    "--disable-blink-features=AutomationControlled",
                    "--no-sandbox",
                ]
            )
            
            context = browser.new_context(
                viewport={"width": 1920, "height": 1080},
                user_agent="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
                locale="en-US",
            )
            
            page = context.new_page()
            
            # Visit the site
            page.goto(url, wait_until="networkidle", timeout=30000)
            
            # Wait a bit for any JS to set cookies
            page.wait_for_timeout(2000)
            
            # Get all cookies
            cookies = context.cookies()
            
            # Save cookies
            save_cookies(cookies)
            
            # Convert to dict
            cookies_dict = {c["name"]: c["value"] for c in cookies}
            
            logger.info(f"Harvested {len(cookies_dict)} cookies")
            print(f"Successfully harvested {len(cookies_dict)} cookies")
            
            browser.close()
            
    except Exception as e:
        logger.error(f"Failed to harvest cookies: {e}")
        print(f"Error harvesting cookies: {e}")
    
    return cookies_dict


def get_cookies_for_url(url: str) -> dict[str, str]:
    """Get cookies for a URL, harvesting if necessary.
    
    Parameters
    ----------
    url
        The target URL to get cookies for.
        
    Returns
    -------
    dict[str, str]
        Dictionary of cookie name -> value pairs.
    """
    cookies = load_cookies()
    
    if not cookies:
        logger.info("No cookies found, harvesting...")
        cookies = harvest_cookies(url)
    
    return cookies


def clear_cookies() -> None:
    """Clear stored cookies."""
    if COOKIE_FILE.exists():
        os.remove(COOKIE_FILE)
        logger.info("Cookies cleared")
        print("Cookies cleared")
