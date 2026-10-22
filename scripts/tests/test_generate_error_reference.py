#!/usr/bin/env python3
"""Tests for `scripts/generate-error-reference.py` (issue #531).

The generator is the thing that keeps README.md's "Error Reference" section
honest, so it gets its own tests.  They cover two things:

  * the parsing itself — the enum is read completely, duplicates are detected,
    raise sites are attributed to the right contract entry points;
  * the drift detection — `--check` fails when README.md no longer matches the
    enum, and `--write` repairs it.

Run with:

    python3 -m unittest discover -s scripts/tests -v

No third-party dependencies: the tests only use the standard library so they
can run anywhere the generator itself runs.
"""

from __future__ import annotations

import importlib.util
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SCRIPT_PATH = REPO_ROOT / "scripts" / "generate-error-reference.py"
RUST_FILE = REPO_ROOT / "intent_settlement" / "src" / "lib.rs"
README_FILE = REPO_ROOT / "README.md"


def load_generator():
    """Import the generator module (its filename is not a valid identifier)."""
    spec = importlib.util.spec_from_file_location("generate_error_reference", SCRIPT_PATH)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


gen = load_generator()


class EnumParsingTests(unittest.TestCase):
    """The `Error` enum is read in full, including its duplicate declarations."""

    @classmethod
    def setUpClass(cls):
        cls.src = RUST_FILE.read_text()
        cls.variants = gen.parse_error_enum(cls.src)

    def test_every_declaration_is_parsed(self):
        # `awk '/pub enum Error/,/^}/'` on the same file reports 127 variants.
        self.assertEqual(len(self.variants), 127)

    def test_indices_are_sequential(self):
        self.assertEqual(
            [v.index for v in self.variants], list(range(1, len(self.variants) + 1))
        )

    def test_codes_are_parsed(self):
        by_name = {v.name: v for v in self.variants}
        self.assertEqual(by_name["AlreadyInitialized"].code, 1)
        self.assertEqual(by_name["Unauthorized"].code, 2)
        self.assertEqual(by_name["IntentNotFound"].code, 3)

    def test_rustdoc_is_parsed(self):
        by_name = {v.name: v for v in self.variants}
        self.assertIn("already has an `Admin` key", by_name["AlreadyInitialized"].condition)

    def test_duplicate_declarations_are_kept_separate(self):
        # The enum declares `IntentNotOpen` twice (codes 4 and 63/64); the
        # parser must not silently collapse them.
        names = [v.name for v in self.variants]
        self.assertGreater(len(names), len(set(names)))

        by_name: dict[str, list] = {}
        for variant in self.variants:
            by_name.setdefault(variant.name, []).append(variant)
        duplicated = {n: g for n, g in by_name.items() if len(g) > 1}
        self.assertEqual(len(duplicated), 32)
        self.assertIn("InvalidConfig", duplicated)

    def test_reused_discriminants_are_detectable(self):
        by_code: dict[int, list[str]] = {}
        for variant in self.variants:
            by_code.setdefault(variant.code, []).append(variant.name)
        reused = {c: n for c, n in by_code.items() if len(n) > 1}
        self.assertEqual(len(reused), 38)


class RaiseSiteTests(unittest.TestCase):
    """Raise sites are attributed to the contract entry points that surface them."""

    @classmethod
    def setUpClass(cls):
        cls.src = RUST_FILE.read_text()
        cls.variants, cls.surfacing = gen.analyse(cls.src)

    def by_name(self):
        return {v.name: v for v in self.variants}

    def test_entry_points_are_discovered(self):
        entries = gen.contract_entry_points(self.src)
        for expected in (
            "initialize",
            "submit_intent",
            "accept_intent",
            "fill_intent",
            "cancel_intent",
            "expire_intent",
            "slash_solver",
        ):
            self.assertIn(expected, entries)

    def test_direct_raise_is_attributed_to_its_entry_point(self):
        self.assertEqual(self.by_name()["AlreadyInitialized"].entries, ["initialize"])
        self.assertEqual(self.by_name()["RescueProtectedToken"].entries, ["rescue_tokens"])
        self.assertEqual(
            self.by_name()["NoPendingFeeRecipient"].entries,
            ["accept_fee_recipient", "cancel_pending_fee_recipient"],
        )

    def test_helper_only_error_is_attributed_through_the_call_graph(self):
        # `ContractPaused` is raised inside `require_not_paused`, so it is
        # surfaced by the entry points that call that helper — not by a
        # helper-only row.
        entries = self.by_name()["ContractPaused"].entries
        self.assertIn("submit_intent", entries)
        self.assertIn("accept_intent", entries)
        self.assertIn("fill_intent", entries)
        self.assertFalse(self.by_name()["ContractPaused"].internal_only)

    def test_variants_raised_only_by_internals_are_marked(self):
        variant = self.by_name()["InvalidTokenInterface"]
        self.assertTrue(variant.never_raised)

    def test_undeclared_variants_are_reported(self):
        # The code raises these but the enum never declares them (#567).
        self.assertIn("NoPendingChainOnboarding", self.surfacing)
        self.assertIn("OutboundIntentNotFound", self.surfacing)
        self.assertIn("IntentNotAmendable", self.surfacing)


class RenderTests(unittest.TestCase):
    """The rendered Markdown covers every declaration."""

    @classmethod
    def setUpClass(cls):
        cls.section, cls.variants, cls.undeclared = gen.build_section(
            RUST_FILE.read_text()
        )

    def test_one_row_per_declaration(self):
        # Only the main table carries a `#` column; the collision tables that
        # follow it are keyed by variant name and by discriminant.
        main_table = self.section.split("| # | Variant | Code | Raised by | Condition |", 1)[1]
        main_table = main_table.split("\n\n⚠ marks a declaration", 1)[0]

        row_numbers = []
        for line in main_table.split("\n"):
            if not line.startswith("| "):
                continue
            cell = line.split("|")[1].strip()
            if cell.isdigit():
                row_numbers.append(int(cell))

        self.assertEqual(row_numbers, list(range(1, len(self.variants) + 1)))

    def test_every_variant_name_appears(self):
        for variant in self.variants:
            self.assertIn(f"`{variant.name}`", self.section)

    def test_collisions_section_is_present(self):
        self.assertIn("#### Declaration collisions in `Error`", self.section)
        self.assertIn("32 variant names are declared more than once", self.section)
        self.assertIn("38 discriminant values are reused", self.section)

    def test_undeclared_variants_are_documented(self):
        self.assertIn("**The code raises variants the enum does not declare:**", self.section)

    def test_markers_are_emitted(self):
        self.assertTrue(self.section.startswith(gen.BEGIN_MARKER))
        self.assertTrue(self.section.endswith(gen.END_MARKER))


class DriftDetectionTests(unittest.TestCase):
    """`--check` fails on drift and `--write` repairs it."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="error-reference-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)

        (self.tmp / "scripts").mkdir()
        shutil.copy(SCRIPT_PATH, self.tmp / "scripts" / SCRIPT_PATH.name)
        (self.tmp / "intent_settlement" / "src").mkdir(parents=True)
        shutil.copy(RUST_FILE, self.tmp / "intent_settlement" / "src" / "lib.rs")
        shutil.copy(README_FILE, self.tmp / "README.md")

    def run_script(self, *args):
        return subprocess.run(
            [sys.executable, str(self.tmp / "scripts" / SCRIPT_PATH.name), *args],
            capture_output=True,
            text=True,
        )

    def test_check_passes_on_a_synced_readme(self):
        result = self.run_script("--check")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_check_fails_when_the_readme_drifts(self):
        readme = self.tmp / "README.md"
        text = readme.read_text()
        readme.write_text(text.replace("| 1 | `AlreadyInitialized` |", "| 1 | `Bogus` |"))

        result = self.run_script("--check")
        self.assertEqual(result.returncode, 1)
        self.assertIn("DRIFT", result.stderr)

    def test_check_fails_when_a_variant_is_dropped_from_the_enum(self):
        rust = self.tmp / "intent_settlement" / "src" / "lib.rs"
        text = rust.read_text()
        rust.write_text(text.replace("    AlreadyInitialized = 1,\n", "", 1))

        result = self.run_script("--check")
        self.assertEqual(result.returncode, 1)
        self.assertIn("DRIFT", result.stderr)

    def test_write_repairs_drift(self):
        readme = self.tmp / "README.md"
        text = readme.read_text()
        readme.write_text(text.replace("| 1 | `AlreadyInitialized` |", "| 1 | `Bogus` |"))

        result = self.run_script("--write")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("`AlreadyInitialized`", readme.read_text())

        again = self.run_script("--check")
        self.assertEqual(again.returncode, 0, again.stderr)

    def test_write_is_idempotent(self):
        before = (self.tmp / "README.md").read_text()
        self.run_script("--write")
        self.assertEqual(before, (self.tmp / "README.md").read_text())

    def test_write_preserves_the_rest_of_the_readme(self):
        readme = self.tmp / "README.md"
        original = readme.read_text()
        self.run_script("--write")
        updated = readme.read_text()

        head = original[: original.index(gen.BEGIN_MARKER)]
        tail = original[original.index(gen.END_MARKER) + len(gen.END_MARKER) :]
        self.assertTrue(updated.startswith(head))
        self.assertTrue(updated.endswith(tail))


if __name__ == "__main__":
    unittest.main()
