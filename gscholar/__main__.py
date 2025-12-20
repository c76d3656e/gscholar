"""gscholar's CLI module with 3-stage pipeline."""

import argparse
import csv
import logging
import os
import sys
from datetime import datetime

import gscholar as gs

logger = logging.getLogger("gscholar")
logging.basicConfig(
    format="%(asctime)s %(levelname)s %(name)s %(message)s",
    level=logging.WARNING,
)


def save_csv(filepath: str, data: list[dict], priority_fields: list[str] | None = None) -> None:
    """Helper to save list of dicts to CSV."""
    if not data:
        print(f"No data to save to {filepath}")
        return
    
    fieldnames = set()
    for item in data:
        fieldnames.update(item.keys())
    
    if priority_fields:
        sorted_fieldnames = [f for f in priority_fields if f in fieldnames] + \
                            sorted([f for f in fieldnames if f not in priority_fields])
    else:
        sorted_fieldnames = sorted(fieldnames)
    
    with open(filepath, 'w', newline='', encoding='utf-8-sig') as csvfile:
        writer = csv.DictWriter(csvfile, fieldnames=sorted_fieldnames)
        writer.writeheader()
        writer.writerows(data)
    print(f"Saved: {filepath}")


def main() -> None:
    """CLI entry point."""
    parser = argparse.ArgumentParser(description="Google Scholar CLI with 3-stage pipeline")
    
    # Basic arguments
    parser.add_argument("keyword", metavar='"search terms"', nargs="?", help='Search terms')
    parser.add_argument("-d", "--debug", action="store_true", help="Show debug output")
    parser.add_argument("-o", "--output", dest="output_path", default="./output", help="Output directory (default: ./output)")
    parser.add_argument("--pages", default="1", help="Page range (e.g., 1-10). Default: 1")
    parser.add_argument("--ylo", type=int, help="Year Low filter. Default: current year - 5")
    parser.add_argument("--proxy", help="Proxy URL (e.g., http://127.0.0.1:7890)")
    parser.add_argument("--mirror", help="Mirror site URL")
    
    # Cookie management
    parser.add_argument("--refresh-cookies", action="store_true", help="Refresh cookies")
    parser.add_argument("--clear-cookies", action="store_true", help="Clear cookies")
    
    # EasyScholar filters
    parser.add_argument("--easyscholar-key", help="EasyScholar API key (required for filtering)")
    parser.add_argument("--sciif", type=float, help="Filter: Impact Factor >= value")
    parser.add_argument("--jci", type=float, help="Filter: JCI >= value")
    parser.add_argument("--sci", help="Filter: SCI partition (e.g., 'Q1')")
    parser.add_argument("--sciUpTop", help="Filter: sciUpTop (substring match)")
    parser.add_argument("--sciBase", help="Filter: sciBase (substring match)")
    parser.add_argument("--sciUp", help="Filter: sciUp (substring match)")
    
    parser.add_argument("--version", action="version", version=gs.__VERSION__)
    
    args = parser.parse_args()
    
    if args.debug:
        logger.setLevel(logging.DEBUG)

    # Handle cookie management
    from gscholar.cookies import clear_cookies, harvest_cookies
    
    if args.clear_cookies:
        clear_cookies()
        print("Cookies cleared.")
        if not args.keyword:
            sys.exit(0)
    
    if args.refresh_cookies:
        harvest_cookies(args.mirror or "https://scholar.google.com")
        if not args.keyword:
            sys.exit(0)
    
    if not args.keyword:
        print("Error: search terms required.")
        sys.exit(1)

    # Parse pages
    try:
        if "-" in args.pages:
            start_p, end_p = map(int, args.pages.split("-"))
            pages = list(range(start_p, end_p + 1))
        else:
            pages = [int(args.pages)]
    except ValueError:
        print("Error: Invalid --pages format.")
        sys.exit(1)

    ylo_val = args.ylo if args.ylo else datetime.now().year - 5

    # --- Create output folder ---
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    safe_keyword = "".join([c for c in args.keyword if c.isalnum() or c in (' ', '-', '_')]).strip().replace(' ', '_')
    output_folder = os.path.join(args.output_path, f"{timestamp}_{safe_keyword}")
    os.makedirs(output_folder, exist_ok=True)
    print(f"Output folder: {output_folder}")

    # ===========================================
    # STAGE 1: Google Scholar Scrape
    # ===========================================
    print("\n--- Stage 1: Google Scholar ---")
    biblist = gs.query(
        args.keyword,
        gs.FORMAT_BIBTEX,
        True,  # allresults
        proxy=args.proxy,
        pages=pages,
        ylo=ylo_val,
        base_url=args.mirror,
    )
    
    if not biblist:
        print("No results from Google Scholar.")
        sys.exit(1)
    
    print(f"Found {len(biblist)} results from Google Scholar.")
    
    gscholar_path = os.path.join(output_folder, "1_gscholar.csv")
    save_csv(gscholar_path, biblist, ['title', 'author', 'year', 'venue', 'article_url', 'citations', 'snippet'])

    # ===========================================
    # STAGE 2: Crossref Enrichment
    # ===========================================
    print("\n--- Stage 2: Crossref Enrichment ---")
    from gscholar.crossref import CrossrefClient
    crossref_client = CrossrefClient(max_workers=3)
    
    # Extract all titles for batch lookup
    titles = [item.get("title", "") for item in biblist]
    print(f"Looking up {len(titles)} titles (concurrent, 3 workers)...")
    
    crossref_results = crossref_client.lookup_batch(titles)
    
    enriched_list = []
    for i, item in enumerate(biblist):
        crossref_data = crossref_results[i]
        
        enriched_item = item.copy()
        if crossref_data:
            enriched_item["doi"] = crossref_data.get("doi", "")
            enriched_item["journal"] = crossref_data.get("journal", "")
            enriched_item["crossref_authors"] = crossref_data.get("authors", "")
            enriched_item["crossref_date"] = crossref_data.get("date", "")
            enriched_item["abstract"] = crossref_data.get("abstract", "")
        else:
            enriched_item["doi"] = ""
            enriched_item["journal"] = ""
            enriched_item["crossref_authors"] = ""
            enriched_item["crossref_date"] = ""
            enriched_item["abstract"] = ""
        
        enriched_list.append(enriched_item)
    
    print(f"Crossref: {sum(1 for r in crossref_results if r)} / {len(titles)} matched")
    
    crossref_path = os.path.join(output_folder, "2_crossref.csv")
    save_csv(crossref_path, enriched_list, ['title', 'doi', 'journal', 'author', 'crossref_authors', 'crossref_date', 'abstract', 'article_url', 'citations'])

    # ===========================================
    # STAGE 3: EasyScholar Filtering
    # ===========================================
    filter_active = any([
        args.sciif is not None,
        args.jci is not None,
        args.sci,
        args.sciUpTop,
        args.sciBase,
        args.sciUp
    ])
    
    if filter_active:
        print("\n--- Stage 3: EasyScholar Filtering ---")
        
        if not args.easyscholar_key:
            print("Error: --easyscholar-key required for filtering.")
            sys.exit(1)
        
        from gscholar.rankings import RankingClient
        ranking_client = RankingClient(args.easyscholar_key)
        
        filtered_list = []
        for item in enriched_list:
            journal = item.get("journal", "")
            if not journal:
                logger.debug(f"No journal for: {item.get('title')}")
                continue
            
            rank_data = ranking_client.get_rank(journal)
            if not rank_data:
                logger.debug(f"No ranking for journal: {journal}")
                continue
            
            # Get metrics
            sciif_val = ranking_client.get_metric(rank_data, "sciif")
            jci_val = ranking_client.get_metric(rank_data, "jci")
            sci_val = ranking_client.get_metric(rank_data, "sci")
            sciUpTop_val = ranking_client.get_metric(rank_data, "sciUpTop")
            sciBase_val = ranking_client.get_metric(rank_data, "sciBase")
            sciUp_val = ranking_client.get_metric(rank_data, "sciUp")
            
            # Check filters
            keep = True
            
            if args.sciif is not None:
                try:
                    if sciif_val is None or float(sciif_val) < args.sciif:
                        keep = False
                except ValueError:
                    keep = False
            
            if keep and args.jci is not None:
                try:
                    if jci_val is None or float(jci_val) < args.jci:
                        keep = False
                except ValueError:
                    keep = False
            
            if keep and args.sci:
                if sci_val is None or args.sci not in str(sci_val):
                    keep = False
            
            if keep and args.sciUpTop:
                if sciUpTop_val is None or args.sciUpTop not in str(sciUpTop_val):
                    keep = False
            
            if keep and args.sciBase:
                if sciBase_val is None or args.sciBase not in str(sciBase_val):
                    keep = False
            
            if keep and args.sciUp:
                if sciUp_val is None or args.sciUp not in str(sciUp_val):
                    keep = False
            
            if keep:
                # Add ranking columns
                item["IF"] = sciif_val or ""
                item["JCI"] = jci_val or ""
                item["SCI"] = sci_val or ""
                item["sciUpTop"] = sciUpTop_val or ""
                item["sciBase"] = sciBase_val or ""
                item["sciUp"] = sciUp_val or ""
                filtered_list.append(item)
        
        print(f"Filtered: {len(filtered_list)} / {len(enriched_list)}")
        
        easyscholar_path = os.path.join(output_folder, "3_easyscholar.csv")
        save_csv(easyscholar_path, filtered_list, ['title', 'IF', 'JCI', 'SCI', 'journal', 'doi', 'author', 'abstract', 'article_url'])
    else:
        print("\n--- Stage 3: Skipped (no filters specified) ---")

    print(f"\n✓ Pipeline complete. Results in: {output_folder}")


if __name__ == "__main__":
    main()
