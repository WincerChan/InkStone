#!/usr/bin/env python3
import argparse
import os
import shutil
import subprocess
import sys
from typing import Iterable, List, Optional, Tuple


BATCH_SIZE = 500


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Import legacy likes.sql data into kudos."
    )
    parser.add_argument(
        "--input",
        default="likes.sql",
        help="Path to likes.sql file.",
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
    return parser.parse_args()


def sql_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def load_insert_block(contents: str) -> str:
    marker = "INSERT INTO"
    idx = contents.find(marker)
    if idx < 0:
        return ""
    start = contents.find("VALUES", idx)
    if start < 0:
        return ""
    values = contents[start + len("VALUES") :].strip()
    if values.endswith(";"):
        values = values[:-1].strip()
    return values


def split_tuples(values: str) -> Iterable[str]:
    tuples = []
    depth = 0
    in_string = False
    current = []
    i = 0
    while i < len(values):
        ch = values[i]
        if in_string:
            current.append(ch)
            if ch == "'":
                if i + 1 < len(values) and values[i + 1] == "'":
                    current.append(values[i + 1])
                    i += 1
                else:
                    in_string = False
            i += 1
            continue
        if ch == "'":
            in_string = True
            current.append(ch)
            i += 1
            continue
        if ch == "(":
            depth += 1
            if depth == 1:
                current = []
            else:
                current.append(ch)
            i += 1
            continue
        if ch == ")":
            if depth == 1:
                tuples.append("".join(current).strip())
                depth = 0
                current = []
            else:
                depth -= 1
                current.append(ch)
            i += 1
            continue
        if depth >= 1:
            current.append(ch)
        i += 1
    return tuples


def split_fields(value: str) -> List[str]:
    fields: List[str] = []
    in_string = False
    current = []
    i = 0
    while i < len(value):
        ch = value[i]
        if in_string:
            current.append(ch)
            if ch == "'":
                if i + 1 < len(value) and value[i + 1] == "'":
                    current.append(value[i + 1])
                    i += 1
                else:
                    in_string = False
            i += 1
            continue
        if ch == "'":
            in_string = True
            current.append(ch)
            i += 1
            continue
        if ch == ",":
            fields.append("".join(current).strip())
            current = []
            i += 1
            continue
        current.append(ch)
        i += 1
    if current:
        fields.append("".join(current).strip())
    return fields


def parse_sql_literal(value: str) -> Optional[str]:
    trimmed = value.strip()
    if not trimmed or trimmed.upper() == "NULL":
        return None
    if trimmed.startswith("'") and trimmed.endswith("'"):
        inner = trimmed[1:-1]
        return inner.replace("''", "'")
    return trimmed


def parse_likes(contents: str) -> List[Tuple[str, str, Optional[str]]]:
    values = load_insert_block(contents)
    if not values:
        return []
    rows = []
    for tuple_str in split_tuples(values):
        fields = split_fields(tuple_str)
        if len(fields) < 2:
            continue
        visitor = parse_sql_literal(fields[0]) or ""
        path = parse_sql_literal(fields[1]) or ""
        ts = parse_sql_literal(fields[2]) if len(fields) > 2 else None
        if not visitor or not path:
            continue
        rows.append((path, visitor, ts))
    return rows


def build_sql(rows: List[Tuple[str, str, Optional[str]]]) -> str:
    statements = ["BEGIN;"]
    for i in range(0, len(rows), BATCH_SIZE):
        batch = rows[i : i + BATCH_SIZE]
        values = []
        for path, visitor, ts in batch:
            created_at = "NOW()"
            if ts:
                created_at = f"{sql_quote(ts)}::timestamptz"
            values.append(
                "("
                f"{sql_quote(path)}, "
                f"{sql_quote(visitor)}, "
                f"{created_at}"
                ")"
            )
        statements.append(
            "INSERT INTO kudos (path, interaction_id, created_at) VALUES\n"
            + ",\n".join(values)
            + "\nON CONFLICT (path, interaction_id) DO NOTHING;"
        )
    statements.append("COMMIT;")
    return "\n".join(statements) + "\n"


def main() -> None:
    args = parse_args()
    contents = open(args.input, "r", encoding="utf-8").read()
    rows = parse_likes(contents)
    if not rows:
        raise SystemExit("no likes rows found in input")
    sql = build_sql(rows)

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
