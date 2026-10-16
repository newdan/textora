"""Regression tests for incomplete or stale audit evidence being accepted."""
import importlib.util
import json
import tempfile
import unittest
import plistlib
import sys
import uuid
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "editing-audit.py"
SPEC = importlib.util.spec_from_file_location("editing_audit", SCRIPT)
AUDIT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(AUDIT)
REPOSITORY = SCRIPT.parent.parent
PHASES = ("build", "formatting", "semantic", "file_roundtrip", "history", "harness", "full_verify", "visual_metrics")


class CanvasComparisonTests(unittest.TestCase):
    def test_blinking_caret_over_text_does_not_change_layout(self):
        changed_pixels = [frozenset(range(14))] * 3
        caret_pixels = [frozenset(range(14))] * 3
        self.assertEqual(AUDIT.changed_content_rows(changed_pixels, caret_pixels), 0)

    def test_content_change_outside_both_carets_remains_visible(self):
        changed_pixels = [frozenset(range(40))] * 3
        caret_pixels = [frozenset(range(2)) | frozenset(range(8, 10))] * 3
        self.assertEqual(AUDIT.changed_content_rows(changed_pixels, caret_pixels), 3)

    def test_different_canvas_heights_are_rejected(self):
        with self.assertRaises(ValueError):
            AUDIT.changed_content_rows([], [frozenset()])


class EvidenceAcceptanceTests(unittest.TestCase):
    def setUp(self):
        self.workspace = tempfile.TemporaryDirectory()
        self.addCleanup(self.workspace.cleanup)
        self.directory = Path(self.workspace.name)
        self.identifier = "paragraph-paragraph-enter-1"
        self.manifest = {"run_id": "current", "cases": [{"id": self.identifier, "keys": ["Return", "BackSpace"]}],
                         "required_native_checks": [{"id": identifier, **contract} for identifier, contract in AUDIT.REQUIRED_NATIVE_CHECKS.items()]}
        self.write_json("manifest.json", self.manifest)
        for phase in PHASES:
            (self.directory / f"{phase}.status").write_text("0\n")
        self.captures = self.directory / "screenshots" / self.identifier
        self.captures.mkdir(parents=True)
        for stage in ("position-source", "position-markdown", "before", "step-01", "step-02", "after", "undo", "redo"):
            (self.captures / f"{stage}.png").write_bytes(b"recorded screenshot")
        self.result = {"run_id": "current", "id": self.identifier, "passed": True, "replayPath": "/current/attempt.md"}
        self.write_json(f"screenshots/{self.identifier}/result.json", self.result)
        self.review = {"run_id": "current", "completed": True, "reviewed_cases": [
            {"id": self.identifier, "evidence_fingerprint": AUDIT.case_fingerprint(self.captures), "layout_passed": True,
             "conclusion": "Reviewed every stage and intermediate frame"}]}
        self.write_json("visual-review.json", self.review)
        self.native_review = {"run_id": "current", "build": None, "completed": True, "reviewed_checks": []}
        for identifier, contract in AUDIT.REQUIRED_NATIVE_CHECKS.items():
            captures = self.directory / "native-checks" / identifier
            captures.mkdir(parents=True)
            for stage in contract["stages"]:
                (captures / f"{stage}.png").write_bytes(b"native screenshot fixture")
                (captures / f"{stage}.ax.txt").write_text("observed native state fixture")
            self.write_json(f"native-checks/{identifier}/result.json", {
                "run_id": "current", "id": identifier, "build": None, "passed": True, "replayValid": True,
                "observations": [{"stage": stage} for stage in contract["stages"]],
                "assertions": [{"name": "saved-source", "actual": "fixture source", "expected": "fixture source"}],
            })
            self.native_review["reviewed_checks"].append({"id": identifier, "evidence_fingerprint": AUDIT.case_fingerprint(captures),
                                                         "layout_passed": True, "conclusion": "Reviewed native fixture evidence"})
        self.write_json("native-review.json", self.native_review)

    def write_json(self, relative_path, value):
        (self.directory / relative_path).write_text(json.dumps(value))

    def summary(self):
        AUDIT.summarize(self.directory, REPOSITORY)
        return json.loads((self.directory / "summary.json").read_text())

    def test_complete_evidence_passes(self):
        summary = self.summary()
        self.assertTrue(summary["passed"])
        self.assertEqual(AUDIT.acceptance_status(summary), 0)

    def test_missing_required_native_ime_check_cannot_pass(self):
        (self.directory / "native-checks" / "system-ime" / "result.json").unlink()
        self.assertFalse(self.summary()["passed"], "keyboard roundtrips cannot replace actual IME evidence")

    def test_removing_native_requirements_from_manifest_cannot_pass(self):
        self.manifest["required_native_checks"] = []
        self.write_json("manifest.json", self.manifest)
        self.assertFalse(self.summary()["passed"])

    def test_native_evidence_from_another_build_cannot_pass(self):
        result_path = self.directory / "native-checks" / "system-ime" / "result.json"
        result = json.loads(result_path.read_text())
        result["build"] = {"executable_sha256": "old-build"}
        result_path.write_text(json.dumps(result))
        self.assertFalse(self.summary()["passed"])

    def test_native_review_of_old_observations_cannot_pass(self):
        (self.directory / "native-checks" / "system-ime" / "commit.ax.txt").write_text("changed observation")
        self.assertFalse(self.summary()["passed"])

    def test_native_missing_candidate_screenshot_cannot_pass(self):
        (self.directory / "native-checks" / "system-ime" / "candidate.png").unlink()
        self.assertFalse(self.summary()["passed"])

    def test_native_assertion_mismatch_fails_even_with_passed_marker(self):
        captures = self.directory / "native-checks" / "system-ime"
        result_path = captures / "result.json"
        result = json.loads(result_path.read_text())
        result["assertions"][0]["actual"] = "unexpected paragraph after IME commit"
        result_path.write_text(json.dumps(result))
        self.native_review["reviewed_checks"][0]["evidence_fingerprint"] = AUDIT.case_fingerprint(captures)
        self.write_json("native-review.json", self.native_review)
        summary = self.summary()
        self.assertFalse(summary["passed"])
        self.assertEqual(AUDIT.acceptance_status(summary), 1)

    def test_missing_required_phase_cannot_pass(self):
        (self.directory / "history.status").unlink()
        summary = self.summary()
        self.assertFalse(summary["passed"])
        self.assertEqual(AUDIT.acceptance_status(summary), 2)

    def test_review_completed_without_case_reviews_cannot_pass(self):
        self.review["reviewed_cases"] = []
        self.write_json("visual-review.json", self.review)
        self.assertFalse(self.summary()["passed"])

    def test_missing_intermediate_screenshot_cannot_pass(self):
        (self.captures / "step-01.png").unlink()
        self.assertFalse(self.summary()["passed"])

    def test_missing_position_screenshot_cannot_pass(self):
        (self.captures / "position-source.png").unlink()
        self.review["reviewed_cases"][0]["evidence_fingerprint"] = AUDIT.case_fingerprint(self.captures)
        self.write_json("visual-review.json", self.review)
        self.assertFalse(self.summary()["passed"])

    def test_image_metrics_require_a_valid_complete_replay(self):
        case = self.manifest["cases"][0]
        self.assertTrue(AUDIT.image_check_ready(self.captures, case, self.result, self.manifest))
        self.result["replayValid"] = False
        self.assertFalse(AUDIT.image_check_ready(self.captures, case, self.result, self.manifest))
        self.result["replayValid"] = True
        (self.captures / "step-01.png").unlink()
        self.assertFalse(AUDIT.image_check_ready(self.captures, case, self.result, self.manifest))

    def test_fast_roundtrip_checks_final_layout_but_still_requires_all_ui_evidence(self):
        identifier = "fast-enter-backspace-lf"
        self.manifest["cases"] = [{"id": identifier, "keys": [{"keys": [
            "Return", "Return", "Return", "BackSpace", "BackSpace", "BackSpace",
        ]}]}]
        self.write_json("manifest.json", self.manifest)
        case = self.manifest["cases"][0]
        unchanged_step = [{"layout_changed": False, "changed_content_rows": 0}]
        self.assertTrue(AUDIT.layout_metrics_passed(case, unchanged_step, restored=True))
        self.assertFalse(AUDIT.layout_metrics_passed(case, unchanged_step, restored=False))
        self.assertFalse(AUDIT.layout_metrics_passed(case, [], restored=True))
        individual_key = {"keys": ["Return"]}
        self.assertFalse(AUDIT.layout_metrics_passed(individual_key, unchanged_step, restored=True))
        self.review.update(completed=False, reviewed_cases=[])
        self.write_json("visual-review.json", self.review)
        # Even a batch still needs its before/after captures; no pixels exist in this fixture.
        with patch.dict(sys.modules, {"PIL": SimpleNamespace(Image=None), "numpy": SimpleNamespace()}):
            self.assertEqual(AUDIT.image_metrics(self.directory), 2)
        metrics = json.loads((self.directory / "visual-metrics.json").read_text())
        self.assertEqual(metrics["cases"], [])
        self.assertEqual(metrics["missing"], [identifier])
        summary = self.summary()
        self.assertFalse(summary["ui_complete"])
        self.assertIn(identifier, summary["missing_evidence"])
        self.assertEqual(AUDIT.acceptance_status(summary), 2)

    def test_contact_setup_can_be_read_without_a_checkpoint_row(self):
        self.assertTrue(AUDIT.fixture_setup_matches({"setupMatches": True, "checkpoints": []}))
        self.assertFalse(AUDIT.fixture_setup_matches({"checkpoints": []}))

    def test_diagnostic_saves_cannot_replace_default_replay(self):
        self.result["saveEachStep"] = True
        self.write_json(f"screenshots/{self.identifier}/result.json", self.result)
        self.review["reviewed_cases"][0]["evidence_fingerprint"] = AUDIT.case_fingerprint(self.captures)
        self.write_json("visual-review.json", self.review)
        summary = self.summary()
        self.assertFalse(summary["passed"])
        self.assertEqual(summary["attempt_totals"]["passed"], 0)
        self.assertEqual(summary["attempt_totals"]["diagnostic"], 1)

    def test_missing_reopen_screenshot_cannot_pass(self):
        self.manifest["cases"][0]["reopen_after"] = True
        self.write_json("manifest.json", self.manifest)
        summary = self.summary()
        self.assertFalse(summary["ui_complete"])
        self.assertIn("reopen.png", summary["missing_evidence"][self.identifier])

    def test_previous_build_evidence_cannot_pass(self):
        self.manifest["bundle"] = {"executable_sha256": "current-build"}
        self.result["build"] = {"executable_sha256": "previous-build"}
        self.write_json("manifest.json", self.manifest)
        self.write_json(f"screenshots/{self.identifier}/result.json", self.result)
        summary = self.summary()
        self.assertFalse(summary["ui_complete"])
        self.assertIn("matching build provenance", summary["missing_evidence"][self.identifier])

    def test_visual_failure_cannot_pass(self):
        self.review["reviewed_cases"][0]["layout_passed"] = False
        self.write_json("visual-review.json", self.review)
        summary = self.summary()
        self.assertFalse(summary["passed"])
        self.assertEqual(AUDIT.acceptance_status(summary), 1)

    def test_result_for_another_case_cannot_complete_manifest(self):
        self.result["id"] = "unrelated-case"
        self.write_json(f"screenshots/{self.identifier}/result.json", self.result)
        self.assertFalse(self.summary()["passed"])

    def test_invalid_replay_cannot_pass(self):
        self.result["replayValid"] = False
        self.write_json(f"screenshots/{self.identifier}/result.json", self.result)
        self.assertFalse(self.summary()["passed"])

    def test_prepared_replay_is_pending_and_cannot_pass(self):
        self.result.update(replayStage="prepared", replayValid=False, passed=False)
        self.write_json(f"screenshots/{self.identifier}/result.json", self.result)
        summary = self.summary()
        self.assertEqual(summary["attempt_totals"]["pending_replay"], 1)
        self.assertEqual(summary["attempt_totals"]["invalid_replay"], 0)
        self.assertEqual(AUDIT.acceptance_status(summary), 2)

    def test_interrupted_attempt_without_metadata_is_counted(self):
        interrupted = self.directory / "attempts" / "interrupted"
        interrupted.mkdir(parents=True)
        (interrupted / "before.png").write_bytes(b"interrupted setup evidence")
        summary = self.summary()
        self.assertEqual(summary["attempt_totals"]["recorded"], 2)
        self.assertEqual(summary["attempt_totals"]["unrecorded"], 1)

    def test_review_of_previous_attempt_cannot_pass(self):
        self.review["reviewed_cases"][0]["evidence_fingerprint"] = "previous-attempt"
        self.write_json("visual-review.json", self.review)
        self.assertFalse(self.summary()["passed"])


class BundleIsolationTests(unittest.TestCase):
    def test_rebuilding_creates_a_separately_identifiable_application(self):
        with tempfile.TemporaryDirectory() as workspace:
            repository = Path(workspace) / "repository"
            executable = repository / "target" / "debug" / "textora"
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b"disposable executable fixture")
            output = Path(workspace) / "audit"
            output.mkdir()
            (output / "manifest.json").write_text(json.dumps({"run_id": str(uuid.uuid4()), "cases": []}))
            AUDIT.bundle(output, repository)
            AUDIT.bundle(output, repository)
            plists = list(output.rglob("Info.plist"))
            self.assertEqual(len(plists), 2, "Rebuilds must not reuse the path of a running bundle")
            identifiers = []
            display_names = []
            for plist_path in plists:
                with plist_path.open("rb") as stream:
                    metadata = plistlib.load(stream)
                    identifiers.append(metadata["CFBundleIdentifier"])
                    display_names.append(metadata["CFBundleName"])
            self.assertEqual(len(set(identifiers)), 2, "macOS must distinguish the two builds")
            self.assertEqual(len(set(display_names)), 2, "Native automation must not resolve both builds by the same app name")


if __name__ == "__main__":
    unittest.main()
