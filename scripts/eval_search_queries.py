#!/usr/bin/env python3
import argparse
import json
import time
import urllib.error
import urllib.parse
import urllib.request
from collections import defaultdict
from pathlib import Path


def normalize_url(url: str) -> str:
    if not url:
        return ""
    if url.startswith("http://") or url.startswith("https://"):
        parsed = urllib.parse.urlparse(url)
        return parsed.path or "/"
    return url


def build_expected_hits(item: dict) -> list[str]:
    raw_hits = item.get("expected_hits")
    hits = []
    if isinstance(raw_hits, list):
        hits.extend(normalize_url(url) for url in raw_hits if url)
    else:
        expected_url = item.get("expected_url", "")
        if expected_url:
            hits.append(normalize_url(expected_url))
    return sorted(set(hit for hit in hits if hit))


def fetch_search(base_url: str, query: str, limit: int, timeout: int) -> dict:
    params = urllib.parse.urlencode({"q": query, "limit": str(limit)})
    url = f"{base_url}/v2/search?{params}"
    with urllib.request.urlopen(url, timeout=timeout) as resp:
        body = resp.read().decode("utf-8")
    return json.loads(body)


def evaluate_endpoint(base_url: str, queries: list[dict], limit: int, timeout: int) -> dict:
    results = []
    for item in queries:
        query = item["query"]
        expected_hits = build_expected_hits(item)
        expected_total = len(expected_hits)
        expected_set = set(expected_hits)
        try:
            response = fetch_search(base_url, query, limit, timeout)
        except (urllib.error.URLError, TimeoutError) as exc:
            results.append(
                {
                    "query": query,
                    "expected_hits": expected_hits,
                    "expected_total": expected_total,
                    "returned": 0,
                    "matched": 0,
                    "precision": 0,
                    "recall": 0,
                    "error": str(exc),
                    "category": item.get("category"),
                    "source_field": item.get("source_field"),
                }
            )
            continue

        hits = response.get("hits", [])
        returned_hits = [normalize_url(hit.get("url", "")) for hit in hits]
        returned_hits = [hit for hit in returned_hits if hit]
        matched_hits = [hit for hit in returned_hits if hit in expected_set]
        matched = len(matched_hits)
        returned = len(returned_hits)
        precision = matched / returned if returned else 0
        recall = matched / expected_total if expected_total else 0
        rank = None
        for idx, hit in enumerate(returned_hits):
            if hit in expected_set:
                rank = idx + 1
                break
        results.append(
            {
                "query": query,
                "expected_hits": expected_hits,
                "expected_total": expected_total,
                "total": response.get("total"),
                "rank": rank,
                "returned": returned,
                "matched": matched,
                "precision": precision,
                "recall": recall,
                "category": item.get("category"),
                "source_field": item.get("source_field"),
            }
        )
        time.sleep(0.02)
    return {"base_url": base_url, "results": results}


def summarize(results: list[dict]) -> dict:
    total = len(results)
    expected_total = sum(item.get("expected_total", 0) for item in results)
    returned_total = sum(item.get("returned", 0) for item in results)
    matched_total = sum(item.get("matched", 0) for item in results)
    precision = matched_total / returned_total if returned_total else 0
    recall = matched_total / expected_total if expected_total else 0

    ranked = [item["rank"] for item in results if item.get("rank") is not None]
    avg_rank = sum(ranked) / len(ranked) if ranked else None

    by_category = defaultdict(
        lambda: {"queries": 0, "expected": 0, "returned": 0, "matched": 0}
    )
    by_field = defaultdict(
        lambda: {"queries": 0, "expected": 0, "returned": 0, "matched": 0}
    )
    for item in results:
        cat = item.get("category", "unknown")
        field = item.get("source_field", "unknown")
        by_category[cat]["queries"] += 1
        by_field[field]["queries"] += 1
        by_category[cat]["expected"] += item.get("expected_total", 0)
        by_field[field]["expected"] += item.get("expected_total", 0)
        by_category[cat]["returned"] += item.get("returned", 0)
        by_field[field]["returned"] += item.get("returned", 0)
        by_category[cat]["matched"] += item.get("matched", 0)
        by_field[field]["matched"] += item.get("matched", 0)

    return {
        "total": total,
        "expected_total": expected_total,
        "returned_total": returned_total,
        "matched_total": matched_total,
        "precision": precision,
        "recall": recall,
        "avg_rank": avg_rank,
        "by_category": by_category,
        "by_field": by_field,
    }


def write_report(report_path: Path, summaries: list[dict]) -> None:
    lines = ["# Search Evaluation Report", ""]
    for summary in summaries:
        base_url = summary["base_url"]
        stats = summary["summary"]
        lines.append(f"## {base_url}")
        lines.append("")
        lines.append(f"- total_queries: {stats['total']}")
        lines.append(f"- expected_total: {stats['expected_total']}")
        lines.append(f"- returned_total: {stats['returned_total']}")
        lines.append(f"- matched_total: {stats['matched_total']}")
        lines.append(f"- precision: {stats['precision']:.2%}")
        lines.append(f"- recall: {stats['recall']:.2%}")
        avg_rank = stats["avg_rank"]
        if avg_rank is not None:
            lines.append(f"- avg_rank: {avg_rank:.2f}")
        else:
            lines.append("- avg_rank: n/a")
        lines.append("")
        lines.append("### By category")
        for cat, data in stats["by_category"].items():
            precision = data["matched"] / data["returned"] if data["returned"] else 0
            recall = data["matched"] / data["expected"] if data["expected"] else 0
            lines.append(
                f"- {cat}: matched {data['matched']}/{data['expected']} (recall {recall:.2%}), "
                f"precision {precision:.2%}"
            )
        lines.append("")
        lines.append("### By source field")
        for field, data in stats["by_field"].items():
            precision = data["matched"] / data["returned"] if data["returned"] else 0
            recall = data["matched"] / data["expected"] if data["expected"] else 0
            lines.append(
                f"- {field}: matched {data['matched']}/{data['expected']} (recall {recall:.2%}), "
                f"precision {precision:.2%}"
            )
        lines.append("")

    report_path.write_text("\n".join(lines), encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--input", default="reports/query_eval.json", help="path to query list"
    )
    parser.add_argument(
        "--ports", default="8080,8081,8082", help="comma-separated ports"
    )
    parser.add_argument("--limit", type=int, default=50)
    parser.add_argument("--timeout", type=int, default=10)
    parser.add_argument("--output", default="reports/query_eval_report.json")
    parser.add_argument("--markdown", default="reports/query_eval_report.md")
    args = parser.parse_args()

    input_path = Path(args.input)
    queries = json.loads(input_path.read_text(encoding="utf-8"))

    summaries = []
    for port in args.ports.split(","):
        port = port.strip()
        if not port:
            continue
        base_url = f"http://localhost:{port}"
        data = evaluate_endpoint(base_url, queries, args.limit, args.timeout)
        summary = summarize(data["results"])
        summaries.append({"base_url": base_url, "summary": summary, "results": data["results"]})

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(summaries, ensure_ascii=False, indent=2), encoding="utf-8")

    markdown_path = Path(args.markdown)
    write_report(markdown_path, summaries)
    print(f"report written to {output_path} and {markdown_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
