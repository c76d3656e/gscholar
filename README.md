# gscholar

Google Scholar 3-Stage Literature Pipeline with EasyScholar ranking filters.

## Features

- **Stage 1**: Scrape Google Scholar using Playwright (anti-detection)
- **Stage 2**: Enrich with Crossref API (DOI, journal, abstract)
- **Stage 3**: Filter by EasyScholar rankings (IF, JCI, SCI partitions)

## Installation

```bash
pip install -e .
playwright install chromium
```

## Quick Start

```bash
# Basic search (10 pages, with filtering)
python -m gscholar "machine learning" --pages 1-10 --easyscholar-key "YOUR_KEY" --sciif 5.0
```

Output is saved to `output/{timestamp}_{query}/`:
- `1_gscholar.csv` — Raw Google Scholar results
- `2_crossref.csv` — Enriched with DOI, journal, abstract
- `3_easyscholar.csv` — Filtered by ranking criteria

## CLI Parameters

| Parameter | Description |
|-----------|-------------|
| `keyword` | Search terms (required) |
| `--pages` | Page range, e.g., `1-10` (default: 1) |
| `--ylo` | Year filter, results from this year onwards |
| `--proxy` | Proxy URL (e.g., `http://127.0.0.1:7890`) |
| `--mirror` | Mirror site URL for blocked regions |
| `--output` | Output directory (default: `./output`) |
| `--debug` | Show debug output |

### EasyScholar Filters

Requires `--easyscholar-key`.

| Parameter | Description |
|-----------|-------------|
| `--easyscholar-key` | EasyScholar API key |
| `--sciif` | Minimum Impact Factor (IF >= value) |
| `--jci` | Minimum JCI |
| `--sci` | SCI partition (e.g., "Q1") |
| `--sciUpTop` | sciUpTop filter |
| `--sciBase` | sciBase filter |
| `--sciUp` | sciUp filter |

### Cookie Management

```bash
# Refresh cookies (fixes 403/429 errors)
python -m gscholar --refresh-cookies

# Clear cookies
python -m gscholar --clear-cookies
```

## Output Columns

### 1_gscholar.csv
`title`, `author`, `year`, `venue`, `article_url`, `citations`, `snippet`

### 2_crossref.csv
Adds: `doi`, `journal`, `crossref_authors`, `crossref_date`, `abstract`

### 3_easyscholar.csv
Adds: `IF`, `JCI`, `SCI`, `sciUpTop`, `sciBase`, `sciUp`

## Python API

```python
import gscholar

results = gscholar.query(
    "machine learning",
    pages=[1, 2, 3],
    ylo=2020,
)

for r in results:
    print(f"{r['title']} - {r['year']}")
```

## Troubleshooting

### 429 Too Many Requests
- Use `--refresh-cookies`
- Use `--mirror` with a mirror site
- Wait 10-30 minutes

### Cookies Location
`~/.gscholar_cookies.json`
