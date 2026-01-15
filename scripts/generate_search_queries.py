#!/usr/bin/env python3
import argparse
import json
import random
import re
from pathlib import Path

FIELD_ORDER = ["title", "subtitle", "content"]
CATEGORY_TARGETS = {
    "single": 15,
    "multi_no_space": 10,
    "multi_space": 10,
    "mixed": 10,
    "english": 5,
}
FIELD_TARGETS = {"title": 18, "subtitle": 14, "content": 18}

STOP_WORDS = {
    "a",
    "an",
    "the",
    "and",
    "or",
    "of",
    "to",
    "in",
    "on",
    "for",
    "with",
    "by",
    "is",
    "are",
    "be",
    "as",
    "at",
    "from",
    "const",
    "let",
    "var",
    "def",
    "type",
    "web",
    "china",
    "sorry",
    "png",
    "jpg",
    "jpeg",
    "gif",
    "html",
    "css",
    "js",
}

CJK_SPLIT_CHARS = set("的了在是和与及而也就都还并或被把让")
CJK_STOP_WORDS = {
    "我的",
    "我们",
    "自己",
    "使用",
    "安装",
    "开始",
    "可以",
    "如何",
    "为什么",
    "事情",
    "内容",
    "模板",
    "之后",
    "之前",
    "现在",
    "这个",
    "那个",
    "这样",
    "一样",
    "今天",
    "明天",
    "昨天",
    "这是",
    "这里",
    "那里",
    "由于",
    "已经",
    "还有",
    "后来",
    "如果",
    "为了",
    "能够",
    "可能",
    "需要",
    "应该",
}
CJK_STOP_SUBSTRINGS = [
    "然后",
    "一个",
    "一些",
    "没有",
    "之后",
    "之前",
    "以后",
    "其实",
    "因此",
    "于是",
]

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
    for token in re.findall(r"[A-Za-z0-9]+", text or ""):
        token = token.lower()
        if len(token) < 2:
            continue
        if token.isdigit():
            continue
        if token in STOP_WORDS:
            continue
        tokens.append(token)
    return tokens


def filter_latin_for_mixed(tokens: list[str]) -> list[str]:
    filtered = [t for t in tokens if len(t) >= 3 and t not in STOP_WORDS]
    return filtered


def extract_cjk_candidates(text: str) -> list[str]:
    candidates = []
    for seq in re.findall(r"[\u3400-\u9FFF]+", text or ""):
        if len(seq) >= 2:
            candidates.append(seq)
        segment = []
        for ch in seq:
            if ch in CJK_SPLIT_CHARS:
                if len(segment) >= 2:
                    candidates.append("".join(segment))
                segment = []
                continue
            segment.append(ch)
        if len(segment) >= 2:
            candidates.append("".join(segment))
    deduped = []
    seen = set()
    for item in candidates:
        if item and item not in seen:
            seen.add(item)
            deduped.append(item)
    filtered = []
    for item in deduped:
        if item in CJK_STOP_WORDS:
            continue
        if any(stop in item for stop in CJK_STOP_SUBSTRINGS):
            continue
        filtered.append(item)
    return filtered


def choose_cjk_term(
    candidates: list[str], min_len: int, max_len: int, rng: random.Random
) -> str | None:
    filtered = [c for c in candidates if min_len <= len(c) <= max_len]
    if not filtered:
        trimmed = []
        for c in candidates:
            if len(c) > max_len:
                trimmed.append(c[:max_len])
        filtered = [c for c in trimmed if len(c) >= min_len]
    if not filtered:
        return None
    return rng.choice(filtered)


def choose_two_cjk_terms(candidates: list[str], rng: random.Random) -> str | None:
    filtered = [c for c in candidates if 2 <= len(c) <= 4]
    if len(filtered) < 2:
        return None
    term1 = rng.choice(filtered)
    term2 = rng.choice(filtered)
    if term1 == term2:
        term2 = rng.choice([c for c in filtered if c != term1] or filtered)
    if term1 == term2:
        return None
    return f"{term1} {term2}"


def choose_field(available: list[str], field_counts: dict, field_targets: dict) -> str | None:
    if not available:
        return None
    scored = []
    for field in available:
        remaining = field_targets.get(field, 0) - field_counts.get(field, 0)
        scored.append((remaining, field))
    scored.sort(reverse=True)
    return scored[0][1]


def prefer_specific_terms(candidates: list[str], freq: dict, max_freq: int = 2) -> list[str]:
    specific = [c for c in candidates if freq.get(c, 0) <= max_freq]
    return specific or candidates


def build_queries(entries: list[dict], seed: int) -> list[dict]:
    rng = random.Random(seed)
    field_counts = {key: 0 for key in FIELD_ORDER}
    queries = []
    seen = set()

    cjk_freq = {}
    for entry in entries:
        for field in FIELD_ORDER:
            candidates = extract_cjk_candidates((entry.get(field) or "").strip())
            for term in candidates:
                cjk_freq[term] = cjk_freq.get(term, 0) + 1

    def add_query(query: str, entry: dict, field: str, category: str) -> bool:
        if not query or query in seen:
            return False
        seen.add(query)
        queries.append(
            {
                "query": query,
                "category": category,
                "expected_url": entry.get("url", ""),
                "entry_index": entry.get("_index"),
                "source_field": field,
                "source_text": (entry.get(field) or "")[:120],
            }
        )
        field_counts[field] += 1
        return True

    def build_entry_index(entries: list[dict]) -> list[dict]:
        indexed = []
        for entry in entries:
            text = "".join(entry.get(field) or "" for field in FIELD_ORDER)
            indexed.append(
                {
                    "url": entry.get("url", ""),
                    "cjk_text": normalize_cjk(text),
                    "latin_tokens": set(extract_latin_tokens(text)),
                }
            )
        return indexed

    def parse_query_terms(query: str) -> tuple[list[str], list[str]]:
        cjk_terms = []
        latin_terms = []
        for token in query.split():
            cjk = normalize_cjk(token)
            if cjk:
                cjk_terms.append(cjk)
            latin_terms.extend(extract_latin_tokens(token))
        return cjk_terms, latin_terms

    def collect_expected_hits(indexed_entries: list[dict], query: str) -> list[str]:
        cjk_terms, latin_terms = parse_query_terms(query)
        expected = []
        for entry in indexed_entries:
            if cjk_terms and any(term not in entry["cjk_text"] for term in cjk_terms):
                continue
            if latin_terms and any(term not in entry["latin_tokens"] for term in latin_terms):
                continue
            url = entry["url"]
            if url:
                expected.append(url)
        return sorted(set(expected))

    def field_text(entry: dict, field: str, suffix: str) -> str:
        if suffix == "raw":
            return (entry.get(field) or "").strip()
        return (entry.get(f"{field}_{suffix}") or "").strip()

    shuffled = entries[:]
    rng.shuffle(shuffled)

    def ensure_category(category: str, count: int) -> None:
        attempts = 0
        while count > 0 and attempts < 20000:
            attempts += 1
            entry = rng.choice(shuffled)
            if category == "english":
                available = [
                    field
                    for field in FIELD_ORDER
                    if extract_latin_tokens(field_text(entry, field, "latin"))
                ]
                field = choose_field(available, field_counts, FIELD_TARGETS)
                if not field:
                    continue
                tokens = extract_latin_tokens(field_text(entry, field, "latin"))
                if not tokens:
                    continue
                query = rng.choice(tokens)
            elif category == "mixed":
                preferred = ["title", "subtitle"]
                available = [
                    field for field in preferred if field_text(entry, field, "raw")
                ]
                if not available:
                    available = [
                        field
                        for field in FIELD_ORDER
                        if field_text(entry, field, "raw")
                    ]
                field = choose_field(available, field_counts, FIELD_TARGETS)
                if not field:
                    continue
                candidates = extract_cjk_candidates(field_text(entry, field, "raw"))
                candidates = prefer_specific_terms(candidates, cjk_freq)
                cjk_term = choose_cjk_term(candidates, 3, 4, rng) or choose_cjk_term(
                    candidates, 2, 3, rng
                )
                if not cjk_term:
                    continue
                latin_tokens = []
                for latin_field in preferred:
                    latin_tokens.extend(
                        extract_latin_tokens(field_text(entry, latin_field, "latin"))
                    )
                latin_tokens = filter_latin_for_mixed(latin_tokens)
                if not latin_tokens:
                    for latin_field in FIELD_ORDER:
                        latin_tokens.extend(
                            extract_latin_tokens(field_text(entry, latin_field, "latin"))
                        )
                    latin_tokens = filter_latin_for_mixed(latin_tokens)
                if not latin_tokens:
                    continue
                latin_term = rng.choice(latin_tokens)
                query = f"{cjk_term} {latin_term}"
            elif category == "multi_space":
                preferred = ["title", "subtitle"]
                available = [
                    field
                    for field in preferred
                    if len(field_text(entry, field, "raw")) >= 4
                ]
                if not available:
                    available = [
                        field
                        for field in FIELD_ORDER
                        if len(field_text(entry, field, "raw")) >= 4
                    ]
                field = choose_field(available, field_counts, FIELD_TARGETS)
                if not field:
                    continue
                candidates = extract_cjk_candidates(field_text(entry, field, "raw"))
                candidates = prefer_specific_terms(candidates, cjk_freq)
                query = choose_two_cjk_terms(candidates, rng)
                if not query:
                    continue
            elif category == "multi_no_space":
                available = [
                    field
                    for field in FIELD_ORDER
                    if len(field_text(entry, field, "raw")) >= 4
                ]
                field = choose_field(available, field_counts, FIELD_TARGETS)
                if not field:
                    continue
                candidates = extract_cjk_candidates(field_text(entry, field, "raw"))
                candidates = prefer_specific_terms(candidates, cjk_freq)
                query = choose_cjk_term(candidates, 4, 7, rng)
                if not query:
                    continue
            else:  # single
                preferred = ["title", "subtitle"]
                available = [
                    field
                    for field in preferred
                    if len(field_text(entry, field, "raw")) >= 2
                ]
                if not available:
                    available = [
                        field
                        for field in FIELD_ORDER
                        if len(field_text(entry, field, "raw")) >= 2
                    ]
                field = choose_field(available, field_counts, FIELD_TARGETS)
                if not field:
                    continue
                candidates = extract_cjk_candidates(field_text(entry, field, "raw"))
                candidates = prefer_specific_terms(candidates, cjk_freq)
                query = choose_cjk_term(candidates, 3, 4, rng) or choose_cjk_term(
                    candidates, 2, 3, rng
                )
                if not query:
                    continue

            if add_query(query, entry, field, category):
                count -= 1

        if count > 0:
            raise RuntimeError(f"unable to satisfy category {category}")

    for idx, entry in enumerate(entries):
        entry["_index"] = idx

    for category, count in CATEGORY_TARGETS.items():
        ensure_category(category, count)

    indexed_entries = build_entry_index(entries)
    for item in queries:
        expected_hits = collect_expected_hits(indexed_entries, item["query"])
        item["expected_hits"] = expected_hits
        item["expected_total"] = len(expected_hits)

    return queries


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument(
        "--input", default="search-index.json", help="path to search-index.json"
    )
    parser.add_argument(
        "--output", default="reports/query_eval.json", help="path to output json"
    )
    args = parser.parse_args()

    input_path = Path(args.input)
    output_path = Path(args.output)

    entries = json.loads(input_path.read_text(encoding="utf-8"))
    queries = build_queries(entries, args.seed)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(queries, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"generated {len(queries)} queries -> {output_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
