#!/usr/bin/env python3
"""Report simple Rust code-quality metrics without external dependencies.

The complexity score is an approximation intended for trend tracking and
review triage. It counts branch/loop constructs, match arms, boolean
operators, and `?` early-return operators inside each function body.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from pathlib import Path
import re
import sys


DEFAULT_ROOTS = ("src", "tests", "examples")
FUNC_RE = re.compile(
    r"^\s*(?:(?:pub(?:\([^)]*\))?|async|const|unsafe|extern\s+\"[^\"]+\"|default)\s+)*"
    r"fn\s+([A-Za-z_][A-Za-z0-9_]*)\b"
)


@dataclass(frozen=True)
class FunctionMetric:
    path: Path
    name: str
    start: int
    end: int
    physical_lines: int
    code_lines: int
    complexity: int
    unsafe_count: int
    unwrap_expect_count: int


def sanitize_lines(lines: list[str]) -> list[str]:
    """Remove comments and string/char bodies while preserving line shape."""
    sanitized: list[str] = []
    in_block_comment = False

    for line in lines:
        out: list[str] = []
        i = 0
        in_string = False
        in_char = False
        escaped = False

        while i < len(line):
            c = line[i]
            nxt = line[i + 1] if i + 1 < len(line) else ""

            if in_block_comment:
                if c == "*" and nxt == "/":
                    in_block_comment = False
                    out.extend("  ")
                    i += 2
                else:
                    out.append(" ")
                    i += 1
                continue

            if in_string:
                if escaped:
                    escaped = False
                elif c == "\\":
                    escaped = True
                elif c == '"':
                    in_string = False
                out.append(" ")
                i += 1
                continue

            if in_char:
                if escaped:
                    escaped = False
                elif c == "\\":
                    escaped = True
                elif c == "'":
                    in_char = False
                out.append(" ")
                i += 1
                continue

            if c == "/" and nxt == "/":
                out.extend(" " * (len(line) - i))
                break
            if c == "/" and nxt == "*":
                in_block_comment = True
                out.extend("  ")
                i += 2
                continue
            if c == '"':
                in_string = True
                out.append(" ")
                i += 1
                continue
            if c == "'":
                in_char = True
                out.append(" ")
                i += 1
                continue

            out.append(c)
            i += 1

        sanitized.append("".join(out))

    return sanitized


def rust_files(roots: list[Path]) -> list[Path]:
    files: list[Path] = []
    for root in roots:
        if root.is_file() and root.suffix == ".rs":
            files.append(root)
        elif root.is_dir():
            files.extend(root.rglob("*.rs"))
    return sorted(files)


def count_complexity(body: str) -> int:
    score = 1
    score += len(re.findall(r"\b(?:if|for|while|loop|match)\b", body))
    score += body.count("=>")
    score += body.count("&&")
    score += body.count("||")
    score += body.count("?")
    return score


def count_code_lines(lines: list[str]) -> int:
    return sum(1 for line in lines if line.strip())


def parse_file(path: Path) -> list[FunctionMetric]:
    raw_lines = path.read_text(encoding="utf-8").splitlines()
    lines = sanitize_lines(raw_lines)
    metrics: list[FunctionMetric] = []
    i = 0

    while i < len(lines):
        match = FUNC_RE.search(lines[i])
        if not match:
            i += 1
            continue

        name = match.group(1)
        start = i
        found_body = False
        brace_depth = 0
        j = i

        while j < len(lines):
            line = lines[j]
            if not found_body:
                semicolon_pos = line.find(";")
                brace_pos = line.find("{")
                if semicolon_pos != -1 and (brace_pos == -1 or semicolon_pos < brace_pos):
                    break
                if brace_pos != -1:
                    found_body = True

            if found_body:
                brace_depth += line.count("{")
                brace_depth -= line.count("}")
                if brace_depth == 0:
                    body_lines = lines[start : j + 1]
                    body = "\n".join(body_lines)
                    metrics.append(
                        FunctionMetric(
                            path=path,
                            name=name,
                            start=start + 1,
                            end=j + 1,
                            physical_lines=j - start + 1,
                            code_lines=count_code_lines(body_lines),
                            complexity=count_complexity(body),
                            unsafe_count=len(re.findall(r"\bunsafe\b", body)),
                            unwrap_expect_count=len(re.findall(r"\.(?:unwrap|expect)\s*\(", body)),
                        )
                    )
                    break
            j += 1

        i = max(j + 1, i + 1)

    return metrics


def rel(path: Path) -> str:
    try:
        return str(path.relative_to(Path.cwd()))
    except ValueError:
        return str(path)


def print_table(title: str, metrics: list[FunctionMetric], field: str, limit: int) -> None:
    print(f"\n{title}")
    print("metric  lines  code  unsafe  unwraps  location")
    print("------  -----  ----  ------  -------  --------")
    for metric in sorted(metrics, key=lambda m: getattr(m, field), reverse=True)[:limit]:
        value = getattr(metric, field)
        print(
            f"{value:>6}  {metric.physical_lines:>5}  {metric.code_lines:>4}  "
            f"{metric.unsafe_count:>6}  {metric.unwrap_expect_count:>7}  "
            f"{rel(metric.path)}:{metric.start} {metric.name}"
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="*", help="Files or directories to scan.")
    parser.add_argument("--check", action="store_true", help="Fail when thresholds are exceeded.")
    parser.add_argument("--max-function-lines", type=int, default=200)
    parser.add_argument("--max-complexity", type=int, default=80)
    parser.add_argument("--top", type=int, default=10, help="Rows to print in each table.")
    args = parser.parse_args()

    roots = [Path(p) for p in (args.paths or DEFAULT_ROOTS)]
    files = rust_files(roots)
    metrics = [metric for path in files for metric in parse_file(path)]

    print("Rust quality metrics")
    print(f"scanned_files: {len(files)}")
    print(f"functions: {len(metrics)}")
    print(f"max_function_lines: {args.max_function_lines}")
    print(f"max_complexity: {args.max_complexity}")

    print_table("Longest functions", metrics, "physical_lines", args.top)
    print_table("Highest estimated complexity", metrics, "complexity", args.top)

    if not args.check:
        return 0

    too_long = [m for m in metrics if m.physical_lines > args.max_function_lines]
    too_complex = [m for m in metrics if m.complexity > args.max_complexity]
    if not too_long and not too_complex:
        print("\nquality-metrics: ok")
        return 0

    print("\nquality-metrics: threshold violation", file=sys.stderr)
    for metric in too_long:
        print(
            f"too long: {rel(metric.path)}:{metric.start} {metric.name} "
            f"({metric.physical_lines} > {args.max_function_lines})",
            file=sys.stderr,
        )
    for metric in too_complex:
        print(
            f"too complex: {rel(metric.path)}:{metric.start} {metric.name} "
            f"({metric.complexity} > {args.max_complexity})",
            file=sys.stderr,
        )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
