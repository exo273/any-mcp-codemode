#!/usr/bin/env python3
"""Validate a commit message against docs/en/commits.md (bilingual EN/ES + Docs links).

Usage:
    check_commit.py <message-file>          # used by .githooks/commit-msg
    check_commit.py --range <rev-range>     # used by CI, e.g. origin/main..HEAD

Stdlib only. Exit code 0 = valid, 1 = invalid, 2 = usage error.
"""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

TYPES = {"feat", "fix", "docs", "refactor", "perf", "test", "build", "ci", "chore", "revert", "style"}
HEADER_RE = re.compile(
    r"^(?P<type>[a-z]+)(?:\((?P<scope>[a-z0-9][a-z0-9._/-]*)\))?(?P<bang>!)?: (?P<summary>\S.*)$"
)
TRAILER_RE = re.compile(r"^(?P<key>[A-Za-z][A-Za-z0-9-]*|BREAKING CHANGE): (?P<value>\S.*)$")
MAX_SUBJECT = 72
SKIP_PREFIXES = ("Merge ", "fixup! ", "squash! ", "amend! ", 'Revert "')
REPO_ROOT = Path(__file__).resolve().parent.parent
LINK_DOC = "docs/en/commits.md | docs/es/commits.md"


def _strip_comments(text: str) -> list[str]:
    lines = [ln.rstrip() for ln in text.replace("\r\n", "\n").split("\n")]
    if "# ------------------------ >8 ------------------------" in lines:
        lines = lines[: lines.index("# ------------------------ >8 ------------------------")]
    lines = [ln for ln in lines if not ln.startswith("#")]
    while lines and not lines[-1]:
        lines.pop()
    while lines and not lines[0]:
        lines.pop(0)
    return lines


def _paragraphs(lines: list[str]) -> list[list[str]]:
    out: list[list[str]] = []
    cur: list[str] = []
    for ln in lines:
        if ln.strip():
            cur.append(ln)
        elif cur:
            out.append(cur)
            cur = []
    if cur:
        out.append(cur)
    return out


def _check_doc_link(value: str) -> str | None:
    if value.startswith("https://"):
        return None
    if value.startswith("http://"):
        return f"Docs link must use https: {value}"
    path = value.split("#", 1)[0]
    if not path.startswith("docs/"):
        return f"Docs link must be https://... or a repo path under docs/: {value}"
    if not (REPO_ROOT / path).exists():
        return f"Docs path does not exist in repo: {path}"
    return None


def validate(message: str) -> list[str]:
    lines = _strip_comments(message)
    if not lines:
        return ["empty commit message"]
    header = lines[0]
    if header.startswith(SKIP_PREFIXES):
        return []

    errors: list[str] = []
    m = HEADER_RE.match(header)
    if not m:
        return [f"header must be '<type>(<scope>)!: <english summary>' — got: {header!r}"]
    if m["type"] not in TYPES:
        errors.append(f"unknown type '{m['type']}', allowed: {', '.join(sorted(TYPES))}")
    summary = m["summary"]
    if len(header) > MAX_SUBJECT:
        errors.append(f"header is {len(header)} chars, max {MAX_SUBJECT}")
    if summary[0].isupper():
        errors.append("english summary must start lowercase (imperative: 'add', 'fix')")
    if summary.endswith("."):
        errors.append("summary must not end with a period")
    if len(lines) > 1 and lines[1]:
        errors.append("line 2 must be blank")

    paras = _paragraphs(lines[1:])
    trailers: list[tuple[str, str]] = []
    if paras and all(TRAILER_RE.match(ln) for ln in paras[-1]):
        trailers = [(TRAILER_RE.match(ln)["key"], TRAILER_RE.match(ln)["value"]) for ln in paras.pop()]

    # Split body into [en] and [es] sections.
    sections: dict[str, list[list[str]]] = {"en": [], "es": []}
    current = None
    for p in paras:
        first = p[0].strip()
        if first in ("[en]", "[es]"):
            current = first[1:3]
            if sections[current]:
                errors.append(f"section [{current}] appears twice")
            rest = p[1:]
            if rest:
                sections[current].append(rest)
            else:
                sections[current].append([])
            continue
        if current is None:
            errors.append("body text must be inside an [en] or [es] section")
            current = "en"
            sections["en"].append([])
        sections[current].append(p)

    es = [p for p in sections["es"] if p]
    en = [p for p in sections["en"] if p]
    if not sections["es"]:
        errors.append("missing [es] section (Spanish summary is required)")
    elif not es:
        errors.append("[es] section is empty: first line must be the Spanish summary")
    else:
        es_summary = es[0][0]
        if len(es[0]) > 1:
            errors.append("[es] summary must be one line followed by a blank line")
        if len(es_summary) > MAX_SUBJECT:
            errors.append(f"[es] summary is {len(es_summary)} chars, max {MAX_SUBJECT}")
        if HEADER_RE.match(es_summary):
            errors.append("[es] summary must not repeat the '<type>(<scope>):' prefix")
        if es_summary.endswith("."):
            errors.append("[es] summary must not end with a period")
    es_body = es[1:]
    if bool(en) != bool(es_body):
        errors.append("body parity: if [en] has a body, [es] needs one after its summary (and vice versa)")

    keys = [k for k, _ in trailers]
    if "Signed-off-by" not in keys:
        errors.append("missing 'Signed-off-by:' trailer (DCO): use 'git commit -s'")
    docs = [v for k, v in trailers if k == "Docs"]
    if not docs:
        errors.append("at least one 'Docs: <https url | docs/... path>' trailer is required")
    for v in docs:
        err = _check_doc_link(v)
        if err:
            errors.append(err)
    for k, v in trailers:
        if k == "Refs" and not re.match(r"^(#\d+|[\w.-]+/[\w.-]+#\d+|https://\S+)(, ?(#\d+|[\w.-]+/[\w.-]+#\d+|https://\S+))*$", v):
            errors.append(f"Refs must be '#123', 'owner/repo#123' or https URLs: {v}")

    breaking = bool(m["bang"]) or "BREAKING CHANGE" in keys
    if breaking:
        if "BREAKING CHANGE" not in keys:
            errors.append("'!' requires a 'BREAKING CHANGE: <english explanation>' trailer")
        es_text = "\n".join("\n".join(p) for p in es_body)
        if "Cambio incompatible:" not in es_text:
            errors.append("breaking change requires a 'Cambio incompatible: ...' paragraph in [es]")
    return errors


def _messages_in_range(rev_range: str) -> list[tuple[str, str]]:
    shas = subprocess.run(
        ["git", "rev-list", "--no-merges", rev_range], capture_output=True, text=True, check=True, cwd=REPO_ROOT
    ).stdout.split()
    out = []
    for sha in shas:
        msg = subprocess.run(
            ["git", "log", "-1", "--format=%B", sha], capture_output=True, text=True, check=True, cwd=REPO_ROOT,
            encoding="utf-8",
        ).stdout
        out.append((sha[:10], msg))
    return out


def main(argv: list[str]) -> int:
    if len(argv) == 2:
        items = [("message", Path(argv[1]).read_text(encoding="utf-8"))]
    elif len(argv) == 3 and argv[1] == "--range":
        items = _messages_in_range(argv[2])
    else:
        print(__doc__, file=sys.stderr)
        return 2
    failed = False
    for ref, msg in items:
        errs = validate(msg)
        if errs:
            failed = True
            print(f"✗ {ref}: commit message invalid / mensaje de commit inválido", file=sys.stderr)
            for e in errs:
                print(f"  - {e}", file=sys.stderr)
    if failed:
        print(f"\nSee / Ver: {LINK_DOC}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
