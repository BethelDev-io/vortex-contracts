#!/usr/bin/env python3
"""Issue #531: generate the README "Error Reference" table from the source.

`intent_settlement`'s `Error` enum is the single source of truth for every
error the contract can raise.  The README's "Error Reference" section used to
carry a hand-written 36-row table whose own caption claimed it mapped *every*
`Error` variant, while the enum actually declares 127 of them.  Roughly 91
real error variants had no documented trigger condition anywhere in the
primary README.  Several of the hand-written rows also pointed at functions
that no longer exist.

This script derives the table from the source instead:

  1. it parses the `pub enum Error { ... }` block out of
     `intent_settlement/src/lib.rs`, capturing every declaration, its
     discriminant and its rustdoc;
  2. it scans the same file for raise sites (`Error::<Variant>`) and works out
     which contract entry points can surface them, by building a call graph of
     the file and walking it from every public entry point of the
     `#[contractimpl]` block;
  3. it renders a Markdown table plus a section describing the declaration
     collisions the enum currently contains.

Because the table is generated, it cannot silently drift from the enum again:
run this script with `--check` and it fails if the README no longer matches
what the source says.

Usage:
  python3 scripts/generate-error-reference.py            # print the section
  python3 scripts/generate-error-reference.py --write    # rewrite README.md
  python3 scripts/generate-error-reference.py --check    # fail on drift
  python3 scripts/generate-error-reference.py --json     # machine-readable dump
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import deque
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RUST_FILE = REPO_ROOT / "intent_settlement" / "src" / "lib.rs"
README_FILE = REPO_ROOT / "README.md"

BEGIN_MARKER = "<!-- BEGIN GENERATED: error-reference -->"
END_MARKER = "<!-- END GENERATED: error-reference -->"

# `pub enum Error {` ... `}` — the closing brace is the first one at column 0.
ENUM_START = re.compile(r"^pub enum Error \{$", re.M)
VARIANT_LINE = re.compile(r"^    ([A-Z][A-Za-z0-9_]*)\s*=\s*(\d+)\s*,$")
DOC_LINE = re.compile(r"^\s*///\s?(.*)$")
SECTION_LINE = re.compile(r"^\s*//\s*[─━\-=]{2,}\s*(.*?)\s*[─━\-=]{2,}\s*$")

# A Rust function definition, at any indentation.
FN_LINE = re.compile(
    r"^(?P<indent>\s*)"
    r"(?P<pub>pub(?:\([A-Za-z_][A-Za-z0-9_]*\))?\s+)?"
    r"(?:async\s+)?fn\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
)
# A block-closing brace on a line of its own.
CLOSE_LINE = re.compile(r"^(?P<indent>\s*)\}")
# `#[contractimpl]` on an `impl` block — the contract's public entry points.
CONTRACT_IMPL = re.compile(r"^#\[contractimpl\]\s*$")
# A call to a plain function or to `Self::function`.
CALL = re.compile(r"(?:Self::)?\b([A-Za-z_][A-Za-z0-9_]*)\s*\(")
# An `Error::Variant` reference — the only way this crate raises a contract error.
RAISE = re.compile(r"Error::([A-Z][A-Za-z0-9_]*)")


class Variant:
    """One `Error` variant declaration."""

    def __init__(self, name: str, code: int, doc: str, section: str, index: int):
        self.name = name
        self.code = code
        self.doc = doc
        self.section = section
        self.index = index
        # Names of the functions whose body raises this variant.
        self.raisers: set[str] = set()
        # Contract entry points that can surface it (filled in by `analyse`).
        self.entries: list[str] = []
        # True when the variant is only reachable through internal helpers.
        self.internal_only: bool = False

    @property
    def condition(self) -> str:
        """The rustdoc, collapsed to a single line."""
        return " ".join(self.doc.split())

    @property
    def never_raised(self) -> bool:
        return not self.raisers


class Function:
    """One function definition, with the lines it spans."""

    def __init__(self, name: str, is_pub: bool, indent: int, start: int):
        self.name = name
        self.is_pub = is_pub
        self.indent = indent
        self.start = start
        self.end = start
        self.callees: set[str] = set()
        self.raises: list[str] = []

    def merge(self, other: "Function") -> None:
        """Fold `other` (a duplicate definition of the same function) into self."""
        self.start = min(self.start, other.start)
        self.end = max(self.end, other.end)
        self.callees |= other.callees
        self.raises.extend(other.raises)


def parse_error_enum(src: str) -> list[Variant]:
    """Return every variant declaration of the `Error` enum, in source order."""
    match = ENUM_START.search(src)
    if match is None:
        raise SystemExit(f"could not find `pub enum Error` in {RUST_FILE}")

    end = src.find("\n}\n", match.end())
    if end == -1:
        raise SystemExit(f"could not find the end of `pub enum Error` in {RUST_FILE}")

    body = src[match.end() : end]

    variants: list[Variant] = []
    doc_lines: list[str] = []
    section = ""

    for line in body.split("\n"):
        section_match = SECTION_LINE.match(line)
        if section_match:
            section = section_match.group(1).strip()
            continue

        doc_match = DOC_LINE.match(line)
        if doc_match:
            doc_lines.append(doc_match.group(1).rstrip())
            continue

        variant_match = VARIANT_LINE.match(line)
        if variant_match:
            variants.append(
                Variant(
                    name=variant_match.group(1),
                    code=int(variant_match.group(2)),
                    doc=" ".join(doc_lines).strip(),
                    section=section,
                    index=len(variants) + 1,
                )
            )
            doc_lines = []
            continue

        # Any other line ends the current doc block.
        if line.strip():
            doc_lines = []

    return variants


def parse_functions(src: str) -> list[Function]:
    """Return every function definition in the file, with its line span.

    The file contains a handful of duplicate definitions (see issue #567), so
    functions are merged by name: the merged view is the union of the
    duplicates' callees and raise sites, which is what the call graph needs.
    """
    lines = src.split("\n")
    functions: list[Function] = []
    stack: list[Function] = []

    for index, line in enumerate(lines):
        fn_match = FN_LINE.match(line)
        if fn_match:
            fn = Function(
                name=fn_match.group("name"),
                is_pub=fn_match.group("pub") is not None,
                indent=len(fn_match.group("indent")),
                start=index,
            )
            functions.append(fn)
            stack.append(fn)

        close_match = CLOSE_LINE.match(line)
        if close_match:
            indent = len(close_match.group("indent"))
            while stack and stack[-1].indent > indent:
                stack[-1].end = index
                stack.pop()
            if stack and stack[-1].indent == indent:
                stack[-1].end = index
                stack.pop()

    for fn in stack:
        fn.end = len(lines)

    merged: dict[str, Function] = {}
    for fn in functions:
        if fn.name in merged:
            merged[fn.name].merge(fn)
        else:
            merged[fn.name] = fn

    return list(merged.values())


def contract_entry_points(src: str) -> set[str]:
    """The `pub fn` names inside every `#[contractimpl]` impl block."""
    lines = src.split("\n")
    entry_points: set[str] = set()

    index = 0
    while index < len(lines):
        if not CONTRACT_IMPL.match(lines[index].strip()):
            index += 1
            continue

        # The next `impl … {` line opens the contract's impl block.
        block_start = index + 1
        depth = 0
        opened = False
        block_end = len(lines) - 1
        for j in range(block_start, len(lines)):
            depth += lines[j].count("{") - lines[j].count("}")
            if "{" in lines[j]:
                opened = True
            if opened and depth <= 0:
                block_end = j
                break

        for j in range(block_start, block_end + 1):
            fn_match = FN_LINE.match(lines[j])
            if fn_match and fn_match.group("pub"):
                entry_points.add(fn_match.group("name"))

        index = block_end + 1

    return entry_points


def build_call_graph(functions: list[Function], src: str) -> None:
    """Fill in each function's callees and the errors it raises."""
    lines = src.split("\n")
    known = {fn.name for fn in functions}

    for fn in functions:
        for index in range(fn.start, min(fn.end + 1, len(lines))):
            line = lines[index]
            stripped = line.strip()
            if stripped.startswith("//"):
                continue
            for callee in CALL.findall(line):
                if callee in known and callee != fn.name:
                    fn.callees.add(callee)
            for variant in RAISE.findall(line):
                fn.raises.append(variant)


def reachable_from(entry_points: set[str], functions: list[Function]) -> dict[str, set[str]]:
    """For each entry point, the set of functions it can (transitively) call."""
    by_name = {fn.name: fn for fn in functions}
    reachable: dict[str, set[str]] = {}

    for entry in sorted(entry_points):
        seen: set[str] = set()
        queue = deque([entry])
        while queue:
            current = queue.popleft()
            fn = by_name.get(current)
            if fn is None:
                continue
            for callee in fn.callees:
                if callee not in seen:
                    seen.add(callee)
                    queue.append(callee)
        reachable[entry] = seen

    return reachable


def analyse(src: str) -> tuple[list[Variant], dict[str, set[str]]]:
    """Parse the enum, work out the raise sites and the surfacing entry points."""
    variants = parse_error_enum(src)
    functions = parse_functions(src)
    build_call_graph(functions, src)

    entry_points = contract_entry_points(src)
    reachable = reachable_from(entry_points, functions)

    # Which functions raise each error variant?
    raisers_by_variant: dict[str, set[str]] = {}
    for fn in functions:
        for variant in fn.raises:
            raisers_by_variant.setdefault(variant, set()).add(fn.name)

    # Which contract entry points can surface each error variant?
    surfacing_by_variant: dict[str, list[str]] = {}
    for name, raisers in raisers_by_variant.items():
        surfacing = sorted(
            entry
            for entry, callees in reachable.items()
            if entry in raisers or (callees & raisers)
        )
        surfacing_by_variant[name] = surfacing

    for variant in variants:
        variant.raisers = raisers_by_variant.get(variant.name, set())
        variant.entries = surfacing_by_variant.get(variant.name, [])
        variant.internal_only = bool(variant.raisers) and not variant.entries

    return variants, surfacing_by_variant


def escape_cell(text: str) -> str:
    """Make `text` safe to drop into a Markdown table cell."""
    return text.replace("\\", "\\\\").replace("|", "\\|").replace("\n", " ")


def format_raised_by(variant: Variant) -> str:
    """Render the "Raised by" cell for one variant."""
    if variant.entries:
        return ", ".join(f"`{name}`" for name in variant.entries)
    if variant.internal_only:
        internal = sorted(name for name in variant.raisers if name not in variant.entries)
        return "_(internal only: " + ", ".join(f"`{n}`" for n in internal) + ")_"
    return "—"


def render(variants: list[Variant], undeclared: dict[str, list[str]]) -> str:
    """Render the generated Markdown section."""
    declared = {v.name for v in variants}

    # Discriminants used by more than one declaration.
    by_code: dict[int, list[str]] = {}
    for variant in variants:
        by_code.setdefault(variant.code, []).append(variant.name)
    reused_codes = {code: names for code, names in by_code.items() if len(names) > 1}

    # Names declared more than once.
    by_name: dict[str, list[Variant]] = {}
    for variant in variants:
        by_name.setdefault(variant.name, []).append(variant)
    duplicated_names = {name: group for name, group in by_name.items() if len(group) > 1}

    never_raised = [v for v in variants if v.never_raised]

    out: list[str] = []
    out.append(BEGIN_MARKER)
    out.append("")
    out.append("This table is **generated** from `intent_settlement/src/lib.rs` by")
    out.append(
        "[`scripts/generate-error-reference.py`](../scripts/generate-error-reference.py)"
    )
    out.append(" — run `python3 scripts/generate-error-reference.py --check` to")
    out.append("verify it is still in sync with the enum.")
    out.append("")
    out.append(
        f"The `Error` enum declares **{len(variants)} variants**"
    )
    out.append(f"(**{len(by_name)} distinct names**). Every declaration gets its own")
    out.append("row below, in the order the enum declares them; the `Code` column")
    out.append("is the on-chain discriminant. The `Condition` column is the")
    out.append("variant's own rustdoc, so the README and the source cannot")
    out.append("disagree about what a variant means.")
    out.append("")
    out.append(
        "`Raised by` lists the contract entry points that can surface the error —"
    )
    out.append("including the ones that only reach it through an internal")
    out.append("helper. It is derived from a call graph of the contract, not from")
    out.append("a hand-written list. A `—` means the variant is declared but")
    out.append("never raised anywhere in `intent_settlement/src/lib.rs`.")
    out.append("")
    out.append("| # | Variant | Code | Raised by | Condition |")
    out.append("|---|---------|------|-----------|-----------|")

    for variant in variants:
        name = f"`{variant.name}`"
        if variant.name in duplicated_names:
            name += " ⚠"
        code = str(variant.code)
        if variant.code in reused_codes:
            code += " ⚠"
        condition = variant.condition or "_(no rustdoc)_"
        out.append(
            "| {idx} | {name} | {code} | {raised} | {cond} |".format(
                idx=variant.index,
                name=escape_cell(name),
                code=escape_cell(code),
                raised=escape_cell(format_raised_by(variant)),
                cond=escape_cell(condition),
            )
        )

    out.append("")
    out.append("⚠ marks a declaration that collides with another one — see")
    out.append("[Declaration collisions](#declaration-collisions-in-error) below.")
    out.append("")

    # ── Collisions ──────────────────────────────────────────────────────────
    out.append("#### Declaration collisions in `Error`")
    out.append("")
    out.append("The enum currently contains duplicate declarations. They are")
    out.append("listed here rather than silently merged, because each")
    out.append("declaration carries its own discriminant and its own rustdoc and")
    out.append("the two are not always equivalent. Fixing them is tracked by")
    out.append(
        "[#567](https://github.com/stellar-vortex-protocol/vortex-contracts/issues/567);"
    )
    out.append("this section exists so the table above stays a faithful")
    out.append("description of the enum as it is today.")
    out.append("")
    out.append(f"**{len(duplicated_names)} variant names are declared more than once:**")
    out.append("")
    out.append("| Variant | Codes | Declarations |")
    out.append("|---------|-------|--------------|")
    for name in sorted(duplicated_names):
        group = duplicated_names[name]
        codes = ", ".join(str(v.code) for v in group)
        docs = " / ".join(
            escape_cell(v.condition) if v.condition else "_(no rustdoc)_"
            for v in group
        )
        out.append(f"| `{name}` | {codes} | {docs} |")
    out.append("")
    out.append(
        f"**{len(reused_codes)} discriminant values are reused by more than one"
    )
    out.append("declaration:**")
    out.append("")
    out.append("| Code | Variants |")
    out.append("|------|---------|")
    for code in sorted(reused_codes):
        names = ", ".join(f"`{n}`" for n in sorted(set(reused_codes[code])))
        out.append(f"| {code} | {names} |")
    out.append("")

    if undeclared:
        out.append("**The code raises variants the enum does not declare:**")
        out.append("")
        out.append("| Variant | Raised by |")
        out.append("|---------|-----------|")
        for name in sorted(undeclared):
            raised = ", ".join(f"`{f}`" for f in undeclared[name])
            out.append(f"| `{name}` | {raised} |")
        out.append("")

    if never_raised:
        out.append(
            f"**{len(never_raised)} declared variants are never raised** in"
        )
        out.append("`intent_settlement/src/lib.rs`:")
        out.append("")
        out.append(", ".join(f"`{v.name}`" for v in never_raised))
        out.append("")
        out.append(
            "Some of these belong to features that are declared but inert — the"
        )
        out.append(
            "operator, delegated-staking, sealed-bid and oracle surfaces are typed"
        )
        out.append(
            "but never reached (see"
        )
        out.append(
            "[#564](https://github.com/stellar-vortex-protocol/vortex-contracts/issues/564)"
        )
        out.append(" and")
        out.append(
            "[#549](https://github.com/stellar-vortex-protocol/vortex-contracts/issues/549))."
        )
        out.append("")

    out.append(END_MARKER)
    return "\n".join(out)


def build_section(src: str) -> tuple[str, list[Variant], dict[str, list[str]]]:
    """Return the rendered section, the parsed variants and the undeclared map."""
    variants, surfacing = analyse(src)

    # Variants the code raises but the enum never declares.
    undeclared: dict[str, list[str]] = {}
    for name, entries in surfacing.items():
        if name not in {v.name for v in variants}:
            undeclared[name] = entries

    return render(variants, undeclared), variants, undeclared


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--write", action="store_true", help="rewrite the section in README.md"
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero if README.md does not match the generated section",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="dump the parsed enum as JSON instead of rendering Markdown",
    )
    args = parser.parse_args()

    src = RUST_FILE.read_text()
    section, variants, undeclared = build_section(src)

    if args.json:
        payload = {
            "variants": [
                {
                    "index": v.index,
                    "name": v.name,
                    "code": v.code,
                    "condition": v.condition,
                    "raised_by": v.entries,
                    "internal_only": v.internal_only,
                    "never_raised": v.never_raised,
                }
                for v in variants
            ],
            "undeclared": undeclared,
        }
        print(json.dumps(payload, indent=2))
        return 0

    if not args.write and not args.check:
        print(section)
        return 0

    readme = README_FILE.read_text()
    start = readme.find(BEGIN_MARKER)
    end = readme.find(END_MARKER)
    if start == -1 or end == -1:
        print(
            f"error: could not find the generated-section markers in {README_FILE}",
            file=sys.stderr,
        )
        return 1

    current = readme[start : end + len(END_MARKER)]
    if current == section:
        print(f"✓ {README_FILE} is in sync with {RUST_FILE}")
        return 0

    if args.check:
        print(
            f"✗ DRIFT: the Error Reference section in {README_FILE} does not match "
            f"{RUST_FILE}.\n"
            f"  Run: python3 scripts/generate-error-reference.py --write",
            file=sys.stderr,
        )
        return 1

    updated = readme[:start] + section + readme[end + len(END_MARKER) :]
    README_FILE.write_text(updated)
    print(f"✓ rewrote the Error Reference section in {README_FILE}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
