#!/usr/bin/env python3
import argparse
import json
from pathlib import Path


def is_cjk_char(ch: str) -> bool:
    code = ord(ch)
    return (
        ch == "\u3007"
        or 0x3400 <= code <= 0x4DBF
        or 0x4E00 <= code <= 0x9FFF
        or 0xF900 <= code <= 0xFAFF
        or 0x20000 <= code <= 0x2A6DF
        or 0x2A700 <= code <= 0x2B73F
        or 0x2B740 <= code <= 0x2B81F
        or 0x2B820 <= code <= 0x2CEAF
        or 0x2F800 <= code <= 0x2FA1F
    )


def normalize_cjk(text: str) -> str:
    if not text:
        return ""
    return "".join(ch for ch in text if is_cjk_char(ch))


def extract_latin_tokens(text: str) -> list[str]:
    tokens = []
    current = []
    for ch in text or "":
        if ch.isascii() and ch.isalnum():
            current.append(ch.lower())
        elif current:
            token = "".join(current)
            tokens.append(token)
            current = []
    if current:
        tokens.append("".join(current))
    return tokens


def build_entry_index(entries: list[dict]) -> list[dict]:
    indexed = []
    for entry in entries:
        title = entry.get("title") or ""
        subtitle = entry.get("subtitle") or ""
        content = entry.get("content") or ""
        headline_text = f"{title}{subtitle}"
        text = f"{title}{subtitle}{content}"
        indexed.append(
            {
                "url": entry.get("url", ""),
                "cjk_text_full": normalize_cjk(text),
                "cjk_text_headline": normalize_cjk(headline_text),
                "latin_tokens": set(extract_latin_tokens(text)),
            }
        )
    return indexed


def parse_query_terms(query: str) -> tuple[list[str], list[str], bool]:
    tokens = query.split()
    has_spaces = len(tokens) > 1
    cjk_terms = []
    latin_terms = []
    for token in tokens:
        cjk = normalize_cjk(token)
        if cjk:
            cjk_terms.append(cjk)
        latin_terms.extend(extract_latin_tokens(token))
    return cjk_terms, latin_terms, has_spaces


def matches_cjk_term(entry: dict, term: str) -> bool:
    if len(term) == 1:
        return term in entry["cjk_text_headline"]
    return term in entry["cjk_text_full"]


def collect_expected_hits(indexed_entries: list[dict], query: str) -> list[str]:
    cjk_terms, latin_terms, has_spaces = parse_query_terms(query)
    if not cjk_terms and not latin_terms:
        return []
    expected = []
    for entry in indexed_entries:
        if has_spaces:
            if cjk_terms and any(not matches_cjk_term(entry, term) for term in cjk_terms):
                continue
            if latin_terms and any(term not in entry["latin_tokens"] for term in latin_terms):
                continue
        else:
            cjk_query = normalize_cjk(query)
            if cjk_query:
                if len(cjk_query) == 1:
                    if cjk_query not in entry["cjk_text_headline"]:
                        continue
                elif cjk_query not in entry["cjk_text_full"]:
                    continue
            if latin_terms and any(term not in entry["latin_tokens"] for term in latin_terms):
                continue
        url = entry["url"]
        if url:
            expected.append(url)
    return sorted(set(expected))


def read_queries(path: Path) -> list[str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    queries = []
    seen = set()
    for line in lines:
        query = line.strip()
        if not query or query in seen:
            continue
        seen.add(query)
        queries.append(query)
    return queries


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--queries", default="query", help="path to query list")
    parser.add_argument(
        "--index", default="search-index.json", help="path to search-index.json"
    )
    parser.add_argument(
        "--output", default="reports/query_eval.json", help="path to output json"
    )
    args = parser.parse_args()

    queries = read_queries(Path(args.queries))
    entries = json.loads(Path(args.index).read_text(encoding="utf-8"))
    indexed_entries = build_entry_index(entries)

    items = []
    for query in queries:
        expected_hits = collect_expected_hits(indexed_entries, query)
        items.append(
            {
                "query": query,
                "category": "real",
                "expected_hits": expected_hits,
                "expected_total": len(expected_hits),
            }
        )

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(items, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"loaded {len(items)} queries -> {output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
