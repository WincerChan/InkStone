#!/usr/bin/env python3
import argparse
import json
import os
import shutil
import subprocess
import sys
from typing import Dict, Iterable, List, Tuple


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Import legacy Disqus comments into comment_items."
    )
    parser.add_argument(
        "--input",
        default="legacy-comments.json",
        help="Path to legacy comments JSON file.",
    )
    parser.add_argument(
        "--db-url",
        default=os.getenv("DATABASE_URL", ""),
        help="Postgres connection string (defaults to DATABASE_URL).",
    )
    parser.add_argument(
        "--execute",
        action="store_true",
        help="Execute SQL via psql instead of printing.",
    )
    parser.add_argument(
        "--output",
        default="",
        help="Write SQL to file instead of stdout (ignored with --execute).",
    )
    parser.add_argument(
        "--source",
        default="legacy",
        help="Value for comment_items.source column.",
    )
    return parser.parse_args()


def sql_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def sql_nullable(value: str) -> str:
    if value is None:
        return "NULL"
    trimmed = value.strip()
    if not trimmed:
        return "NULL"
    return sql_quote(trimmed)


def flatten_comments(nodes: List[dict], parent_id: str = None) -> Iterable[dict]:
    for node in nodes:
        comment_id = str(node.get("id", "")).strip()
        if not comment_id:
            continue
        yield {
            "id": comment_id,
            "parent_id": parent_id,
            "message": node.get("message", ""),
            "author": node.get("author", ""),
            "date": node.get("date", ""),
        }
        children = node.get("children", []) or []
        if children:
            yield from flatten_comments(children, comment_id)


def load_existing_discussions(db_url: str, post_ids: List[str]) -> Dict[str, str]:
    if not db_url or not post_ids:
        return {}
    if not shutil.which("psql"):
        print("psql not found; skipping existing discussion lookup", file=sys.stderr)
        return {}

    quoted = ", ".join(sql_quote(pid) for pid in post_ids)
    sql = (
        "SELECT post_id, discussion_id FROM comment_discussions "
        f"WHERE post_id IN ({quoted});"
    )
    result = subprocess.run(
        ["psql", db_url, "-At", "-F", "\t", "-c", sql],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    mapping: Dict[str, str] = {}
    for line in result.stdout.splitlines():
        if not line.strip():
            continue
        post_id, discussion_id = line.split("\t", 1)
        mapping[post_id] = discussion_id
    return mapping


def build_sql(
    data: dict,
    existing: Dict[str, str],
    source: str,
) -> str:
    statements: List[str] = ["BEGIN;"]

    for post_id, nodes in data.items():
        if not isinstance(nodes, list):
            continue
        comments = list(flatten_comments(nodes))
        if not comments:
            continue

        discussion_id = existing.get(post_id, post_id)

        if discussion_id != post_id:
            statements.append(
                "UPDATE comment_items "
                f"SET discussion_id = {sql_quote(discussion_id)} "
                f"WHERE discussion_id = {sql_quote(post_id)} "
                f"AND source = {sql_quote(source)};"
            )

        dates = [c["date"] for c in comments if c.get("date")]
        created_at = min(dates) if dates else "1970-01-01T00:00:00Z"
        updated_at = max(dates) if dates else created_at

        if post_id not in existing:
            statements.append(
                "INSERT INTO comment_discussions "
                "(post_id, discussion_id, number, title, url, created_at, updated_at) "
                "VALUES ("
                f"{sql_quote(post_id)}, "
                f"{sql_quote(discussion_id)}, "
                "0, "
                f"{sql_quote(post_id)}, "
                "'', "
                f"{sql_quote(created_at)}, "
                f"{sql_quote(updated_at)}"
                ") ON CONFLICT (post_id) DO NOTHING;"
            )

        values = []
        for comment in comments:
            values.append(
                "("
                f"{sql_quote(discussion_id)}, "
                f"{sql_quote(comment['id'])}, "
                f"{sql_nullable(comment.get('parent_id'))}, "
                "'', "
                f"{sql_quote(source)}, "
                f"{sql_nullable(comment.get('author'))}, "
                "NULL, "
                "NULL, "
                f"{sql_quote(comment.get('message', ''))}, "
                f"{sql_quote(comment.get('date', created_at))}, "
                f"{sql_quote(comment.get('date', created_at))}"
                ")"
            )

        statements.append(
            "INSERT INTO comment_items ("
            "discussion_id, comment_id, parent_id, comment_url, source, "
            "author_login, author_url, author_avatar_url, body_html, created_at, updated_at"
            ") VALUES\n"
            + ",\n".join(values)
            + "\nON CONFLICT (discussion_id, comment_id) DO NOTHING;"
        )

    statements.append("COMMIT;")
    return "\n".join(statements) + "\n"


def main() -> None:
    args = parse_args()

    with open(args.input, "r", encoding="utf-8") as handle:
        data = json.load(handle)

    if not isinstance(data, dict):
        raise SystemExit("legacy JSON must be an object keyed by post_id")

    post_ids = [pid for pid in data.keys() if isinstance(pid, str)]
    existing = load_existing_discussions(args.db_url, post_ids)
    sql = build_sql(data, existing, args.source)

    if args.execute:
        if not args.db_url:
            raise SystemExit("--execute requires --db-url or DATABASE_URL")
        if not shutil.which("psql"):
            raise SystemExit("psql not found; cannot execute")
        subprocess.run(
            ["psql", args.db_url, "-v", "ON_ERROR_STOP=1"],
            input=sql,
            text=True,
            check=True,
        )
        return

    if args.output:
        with open(args.output, "w", encoding="utf-8") as handle:
            handle.write(sql)
        return

    sys.stdout.write(sql)


if __name__ == "__main__":
    main()
