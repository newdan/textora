#!/usr/bin/env python3
"""Generate disposable Markdown cases and summarize editing audit evidence."""
import argparse
import json
import hashlib
import subprocess
import plistlib
import shutil
import uuid
from pathlib import Path

BLOCKS = {"paragraph": "before", "rule": "---", "code": "```rust\ncode\n```"}
REQUIRED_PHASES = ("build", "formatting", "semantic", "file_roundtrip", "history", "harness", "full_verify", "visual_metrics")


REQUIRED_NATIVE_CHECKS = {
    "system-ime": {
        "stages": ["preedit", "candidate", "preedit-backspace", "cancel", "commit", "undo", "redo",
                   "code-preedit", "code-commit", "code-undo", "code-redo", "reopen"],
        "expectation": "段落与代码正文中的真实系统输入法；候选、组合退格、取消与提交；确认 Enter 不产生额外换行，历史和重开正确",
    },
    "pointer-boundaries": {
        "stages": ["paragraph", "blank", "rule", "code", "selection", "after", "undo", "redo", "reopen"],
        "expectation": "真实画布点击段落、空段、分割线和代码边界，拖选跨块后替换，核对光标、选区、历史与重开",
    },
    "visual-navigation": {
        "stages": ["wrapped", "up", "down", "home", "end", "after", "undo", "redo", "reopen"],
        "expectation": "自动换行中的上下导航和 Home/End 到达正确视觉位置，再编辑、撤销、重做、重开",
    },
    "resized-wrapped-layout": {
        "stages": ["wide", "narrow", "narrow-edit", "wide-restored", "reopen"],
        "expectation": "真实窗口缩窄与恢复，中英长段和长代码行不重叠截断，缩窗后编辑及重开正确",
    },
    "scrolling-caret-visibility": {
        "stages": ["top", "bottom", "edit", "undo", "redo", "reopen"],
        "expectation": "长文档滚动到首尾并编辑，光标自动可见，撤销重做及重开不丢内容",
    },
    "batch-save-close": {
        "stages": ["dirty", "prompt", "closed", "reopen"],
        "expectation": "多个有修改的独立夹具全部保存关闭，窗口保持响应，保存结果及重开内容正确",
    },
    "lifecycle-focus": {
        "stages": ["before", "other-tab", "source", "markdown", "restart", "reopen"],
        "expectation": "页签往返、源码渲染切换及验收应用重启后焦点和保存内容正确，未误编辑其他文档",
    },
}


NATIVE_LONG_DOCUMENT_SECTION_COUNT = 80
NATIVE_BATCH_FIXTURE_COUNT = 3
NATIVE_PARAGRAPH_REPEAT_COUNT = 8
NATIVE_CODE_REPEAT_COUNT = 12
NATIVE_BLOCK_SOURCE = "前段 English é 👩🏽‍💻\n\n---\n\n```rust\nlet value = 1;\n```\n\n尾段中文与 English。"


def prepare_native_checks(directory, run_id):
    fixture_directory = directory / "native-fixtures" / run_id
    fixture_directory.mkdir(parents=True, exist_ok=True)
    specifications = []
    for identifier, contract in REQUIRED_NATIVE_CHECKS.items():
        source = NATIVE_BLOCK_SOURCE
        if identifier in {"visual-navigation", "resized-wrapped-layout", "scrolling-caret-visibility"}:
            source = "\n\n".join(
                f"段落 {index + 1}：" + "中文与 English é 👩🏽‍💻 自动换行。" * NATIVE_PARAGRAPH_REPEAT_COUNT
                + "\n\n---\n\n```rust\n" + "let long_value = 1; " * NATIVE_CODE_REPEAT_COUNT + "\n```"
                for index in range(NATIVE_LONG_DOCUMENT_SECTION_COUNT)
            )
        if identifier == "lifecycle-focus":
            source = "\ufeff" + source.replace("\n", "\r\n")
        fixture_path = fixture_directory / f"{identifier}.md"
        fixture_path.write_bytes(source.encode("utf-8"))
        specification = {"id": identifier, **contract, "path": str(fixture_path), "source": source}
        if identifier == "batch-save-close":
            specification["group_paths"] = []
            for index in range(NATIVE_BATCH_FIXTURE_COUNT):
                member_path = fixture_directory / f"{identifier}-{index + 1}.md"
                member_path.write_bytes(f"{source}\n\n批量夹具 {index + 1}".encode("utf-8"))
                specification["group_paths"].append(str(member_path))
        specifications.append(specification)
    return specifications


def write_case(case_directory, identifier, source, prefix, keys, expected, expectation):
    path = case_directory / f"{identifier}.md"
    path.write_bytes(source.encode("utf-8"))
    return {"id": identifier, "path": str(path), "source": source,
            "cursor_prefix": prefix, "keys": keys, "expected_source": expected,
            "expectation": expectation, "reopen_after": True}


def archive_previous_run(directory):
    previous_manifest = directory / "manifest.json"
    if not previous_manifest.exists():
        return
    previous_run = str(uuid.UUID(json.loads(previous_manifest.read_text())["run_id"]))
    archive = directory / "archive" / f"{previous_run}-{uuid.uuid4()}"
    archive.mkdir(parents=True)
    for pattern in ["*.log", "*.status", "contact-*.png", "intermediate-*.png", "report.md", "summary.json", "visual-review.*", "native-review.*", "visual-metrics.json", "manifest.json"]:
        for artifact in directory.glob(pattern):
            shutil.move(str(artifact), archive / artifact.name)
    for folder_name in ["screenshots", "attempts", "native-checks"]:
        folder = directory / folder_name
        if folder.exists():
            shutil.move(str(folder), archive / folder_name)


def prepare(directory, repository):
    archive_previous_run(directory)
    run_id = str(uuid.uuid4())
    case_directory = directory / "fixtures" / run_id
    case_directory.mkdir(parents=True, exist_ok=True)
    (directory / "screenshots").mkdir(exist_ok=True)
    cases = []
    for before_kind, before in BLOCKS.items():
        for after_kind, after in BLOCKS.items():
          for ending, newline in [("lf", "\n"), ("crlf", "\r\n")]:
            preceding = before.replace("\n", newline)
            source = f"{preceding}{newline}{newline}{after.replace(chr(10), newline)}"
            for count in range(1, 4):
                identifier = f"{before_kind}-{after_kind}-enter-{count}" + ("-crlf" if ending == "crlf" else "")
                # Enter at the complete preceding block end, followed by matching Backspaces.
                cases.append(write_case(case_directory, identifier, source, preceding,
                    ["Return"] * count + ["BackSpace"] * count, source,
                    "连续 Enter/Backspace 按语义恢复源码、光标与块结构"))
    for ending, newline in [("lf", "\n"), ("crlf", "\r\n")]:
        cases.append(write_case(case_directory, f"rule-delete-{ending}", f"---{newline}after", "---",
            ["Delete"], f"---{newline}after", "分割线末尾 Delete 不得吞入后段"))
        cases.append(write_case(case_directory, f"paragraph-rule-backspace-{ending}", f"before{newline}{newline}---", f"before{newline}{newline}",
            ["BackSpace"], f"before{newline}{newline}---", "不得将分割线转换为 Setext 标题"))
        cases.append(write_case(case_directory, f"tab-rule-backspace-{ending}", f"before{newline}{newline}\t{newline}---", f"before{newline}{newline}\t{newline}",
            ["BackSpace"], f"before{newline}{newline}---", "删除完整空段，不能将 Tab 并到分割线行首"))
        code = f"```rust{newline}code{newline}```{newline}{newline}after"
        cases.append(write_case(case_directory, f"code-first-backspace-{ending}", code, f"```rust{newline}",
            ["BackSpace"], code, "保护开围栏与第一行正文之间的结构换行"))
        cases.append(write_case(case_directory, f"code-close-backspace-{ending}", code, f"```rust{newline}code{newline}",
            ["BackSpace"], code, "保护闭围栏，后段不得变成代码"))
        literal = f"```{newline}# value{newline}```"
        cases.append(write_case(case_directory, f"code-literal-backspace-{ending}", literal, f"```{newline}# ",
            ["BackSpace"], f"```{newline}#value{newline}```", "代码文本只删除一个 grapheme"))
        for separator, name in [(newline, "soft"), (f"\\{newline}", "backslash"), (f"  {newline}", "spaces")]:
            source = f"前{separator}后"
            cases.append(write_case(case_directory, f"existing-{name}-split-{ending}", source, f"前{separator}",
                ["Return", {"paste": "👩🏽‍💻"}, "BackSpace", "BackSpace"], source, "持续实例保留已有换行，输入删空后退格恢复"))
        source = f"```rust{newline}前后{newline}```{newline}{newline}---{newline}{newline}尾段"
        cases.append(write_case(case_directory, f"code-shift-line-break-{ending}", source, f"```rust{newline}前",
            ["shift+Return", "BackSpace"], source, "代码 Shift+Enter 使用字面换行，退格恢复"))
        source = f"前后{newline}{newline}---{newline}{newline}尾段"
        cases.append(write_case(case_directory, f"paragraph-shift-line-break-{ending}", source, "前",
            ["shift+Return", "BackSpace"], source, "段落 Shift+Enter 创建硬换行，退格恢复"))
        source = f"前段{newline}{newline}---{newline}{newline}```rust{newline}code{newline}```{newline}{newline}尾段"
        paste = f"中文👩🏽‍💻{newline}{newline}~~~python{newline}print(1){newline}~~~"
        selected = write_case(case_directory, f"selection-paste-{ending}", source, "", [{"paste": paste}], paste, "真实全选跨块粘贴，撤销恢复原选区文档")
        selected["select_all"] = True
        cases.append(selected)
        source = f"before{newline}{newline}\t{newline}---{newline}{newline}```{newline}code{newline}```"
        cases.append(write_case(case_directory, f"continuous-tab-rule-delete-{ending}", source, "before",
            ["Delete", "Delete", "Delete"], f"before{newline}{newline}---{newline}{newline}```{newline}code{newline}```", "连续 Delete 删除可见 Tab 空段后保护分割线"))
        source = f"> ```rust{newline}> code{newline}> ```"
        cases.append(write_case(case_directory, f"container-code-first-backspace-{ending}", source, f"> ```rust{newline}> ", ["BackSpace"], source, "引用代码首行的退格不删除结构前缀"))
        source = f"> ```rust{newline}> first{newline}> second{newline}> ```"
        cases.append(write_case(case_directory, f"container-code-join-backspace-{ending}", source, f"> ```rust{newline}> first{newline}> ", ["BackSpace"], f"> ```rust{newline}> firstsecond{newline}> ```", "引用代码普通行退格合并字面正文并去掉容器前缀"))
        source = f"before{newline}{newline}after"
        cases.append(write_case(case_directory, f"fast-enter-backspace-{ending}", source, "before", [{"keys": ["Return", "Return", "Return", "BackSpace", "BackSpace", "BackSpace"]}], source, "不插入中途观察的快速连续编辑"))
        cases.append(write_case(case_directory, f"arrow-navigation-edit-{ending}", source, "before", ["Left", "Right", "Return", "BackSpace"], source, "左右导航到达边界后编辑及恢复"))
        source = f"> ```rust{newline}> first{newline}> second{newline}> ```"
        cases.append(write_case(case_directory, f"container-code-forward-join-{ending}", source, f"> ```rust{newline}> first", ["Delete"], f"> ```rust{newline}> firstsecond{newline}> ```", "前向删除合并代码正文，不把引用前缀加入代码"))
        source = f"- ```rust{newline}  first{newline}\tsecond{newline}  ```"
        cases.append(write_case(case_directory, f"code-tab-residue-forward-{ending}", source, f"- ```rust{newline}  first", ["Delete"], f"- ```rust{newline}  first  second{newline}  ```", "前向合并保留 Tab 超过列表缩进的两个字面空格"))
        cases.append(write_case(case_directory, f"code-tab-residue-backward-{ending}", source, f"- ```rust{newline}  first{newline}\t", ["BackSpace"], f"- ```rust{newline}  first{newline}  second{newline}  ```", "退格仅删除 Tab 的字面缩进，保留列表归属"))
        for owner, opening, continuation, chord in [
            ("quote", "> ```rust", "> ", "Return"),
            ("quote-shift", "> ```rust", "> ", "shift+Return"),
            ("list-shift", "- ```rust", "  ", "shift+Return"),
            ("quote-list", "> - ```rust", ">   ", "Return"),
        ]:
            source = f"{opening}{newline}{continuation}first> second{newline}{continuation}```"
            prefix = f"{opening}{newline}{continuation}first"
            cases.append(write_case(case_directory, f"container-code-{owner}-enter-body-{ending}", source, prefix, [chord, "BackSpace"], source, "容器内代码回车保留容器和字面 >，退格恢复"))
        for name, chord in [("paragraph", "Return"), ("shift", "shift+Return")]:
            source = f"> ```rust{newline}> body{newline}> ```"
            expected = f"> ```rust{newline}> {newline}> body{newline}> ```"
            cases.append(write_case(case_directory, f"container-code-opening-{name}-{ending}", source, "> ```rust", [chord], expected, "已有开围栏回车使空代码行继续属于引用"))
        for closed in [True, False]:
            source = f"```rust{newline}body" + (f"{newline}```" if closed else "")
            cases.append(write_case(case_directory, f"code-body-end-enter-{'closed' if closed else 'open'}-{ending}", source, f"```rust{newline}body", ["Return", "BackSpace"], source, "代码末行回车显示新增行，退格恢复代码高度"))
    manifest = {"schema": 2, "run_id": run_id, "repository": str(repository), "cases": cases,
                "required_native_checks": prepare_native_checks(directory, run_id),
                "sample_ids": ["paragraph-rule-enter-1", "rule-paragraph-enter-3", "code-rule-enter-2",
                               "rule-delete-lf", "paragraph-rule-backspace-lf", "tab-rule-backspace-lf",
                               "code-first-backspace-lf", "code-close-backspace-lf", "code-literal-backspace-lf"]}
    (directory / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2))
    print(f"Generated {len(cases)} UI cases in {directory}")


def collect_ui_evidence(directory, manifest):
    completed = []
    missing_evidence = {}
    for case in manifest["cases"]:
        captures = directory / "screenshots" / case["id"]
        result_path = captures / "result.json"
        if not result_path.exists():
            missing_evidence[case["id"]] = ["result.json"]
            continue
        result = json.loads(result_path.read_text())
        if result.get("run_id") != manifest["run_id"] or result.get("id") != case["id"]:
            missing_evidence[case["id"]] = ["matching result identity"]
            continue
        stages = ["before"]
        if result.get("setupMatches", True):
            stages += ["position-source", "position-markdown", "after", "undo", "redo"]
            if case.get("reopen_after"):
                stages.append("reopen")
            stages += [f"step-{index:02}" for index in range(1, len(case["keys"]) + 1)]
        missing = [f"{stage}.png" for stage in stages if not (captures / f"{stage}.png").exists()]
        if result.get("replayValid") is False:
            missing.append("valid native replay")
        if result.get("saveEachStep"):
            missing.append("default replay without per-step Save")
        if manifest.get("bundle") and result.get("build") != manifest["bundle"]:
            missing.append("matching build provenance")
        if missing:
            missing_evidence[case["id"]] = missing
        completed.append(result)
    return completed, missing_evidence


def case_fingerprint(captures):
    digest = hashlib.sha256()
    for artifact in sorted(captures.iterdir()):
        if artifact.suffix not in {".png", ".md", ".txt", ".json"}:
            continue
        digest.update(artifact.name.encode("utf-8") + b"\0")
        digest.update(artifact.read_bytes())
    return digest.hexdigest()


def collect_visual_review(directory, manifest):
    review_path = directory / "visual-review.json"
    review = json.loads(review_path.read_text()) if review_path.exists() else {}
    expected_ids = {case["id"] for case in manifest["cases"]}
    reviewed_cases = review.get("reviewed_cases", [])
    review_ids = {case["id"] for case in reviewed_cases if case.get("conclusion")}
    current_attempts = all((directory / "screenshots" / case["id"]).is_dir()
                           and case.get("evidence_fingerprint") == case_fingerprint(directory / "screenshots" / case["id"])
                           for case in reviewed_cases if case["id"] in expected_ids)
    complete = (review.get("run_id") == manifest["run_id"] and review.get("completed", False)
                and review_ids == expected_ids and len(reviewed_cases) == len(expected_ids) and current_attempts)
    passed = complete and all(case.get("layout_passed", False) for case in reviewed_cases)
    return complete, passed


def collect_native_checks(directory, manifest):
    review_path = directory / "native-review.json"
    review = json.loads(review_path.read_text()) if review_path.exists() else {}
    reviews = review.get("reviewed_checks", [])
    reviewed = {check["id"]: check for check in reviews}
    declarations = {check["id"]: check.get("stages") for check in manifest.get("required_native_checks", [])}
    missing = {}
    if declarations != {identifier: contract["stages"] for identifier, contract in REQUIRED_NATIVE_CHECKS.items()}:
        missing["manifest"] = ["canonical native checklist declaration"]
    if (review.get("run_id") != manifest["run_id"] or review.get("build") != manifest.get("bundle")
            or not review.get("completed") or len(reviewed) != len(reviews)):
        missing["review"] = ["current complete native checklist review"]
    completed = []
    for identifier, contract in REQUIRED_NATIVE_CHECKS.items():
        captures = directory / "native-checks" / identifier
        result_path = captures / "result.json"
        if not result_path.exists():
            missing[identifier] = ["native replay result"]
            continue
        result = json.loads(result_path.read_text())
        absent = [f"{stage}{suffix}" for stage in contract["stages"] for suffix in (".png", ".ax.txt")
                  if not (captures / f"{stage}{suffix}").is_file()]
        if (result.get("run_id") != manifest["run_id"] or result.get("id") != identifier
                or result.get("build") != manifest.get("bundle") or result.get("replayValid") is not True):
            absent.append("matching valid replay provenance")
        if [observation.get("stage") for observation in result.get("observations", [])] != contract["stages"]:
            absent.append("ordered native observations")
        assessment = reviewed.get(identifier, {})
        if (not assessment.get("conclusion") or assessment.get("evidence_fingerprint") != case_fingerprint(captures)):
            absent.append("review bound to current evidence")
        assertions = result.get("assertions", [])
        if not assertions or any(not assertion.get("name") or "actual" not in assertion or "expected" not in assertion
                                 for assertion in assertions):
            absent.append("explicit native behavior assertions")
        if absent:
            missing[identifier] = absent
            continue
        completed.append({**result, "passed": result.get("passed") is True and assessment.get("layout_passed") is True
                          and all(assertion["actual"] == assertion["expected"] for assertion in assertions)})
    return completed, missing


def attempt_outcome(attempt):
    if attempt.get("replayStage") == "preparing":
        return "pending_setup"
    if attempt.get("replayStage") == "prepared":
        return "pending_replay"
    if attempt.get("replayValid") is False:
        return "invalid_replay"
    if attempt.get("setupMatches") is False:
        return "setup_blocked"
    if attempt.get("saveEachStep"):
        return "diagnostic"
    return "passed" if attempt.get("passed", False) else "failed"


def summarize(directory, repository):
    phases = {}
    for path in sorted(directory.glob("*.status")):
        phases[path.stem] = int(path.read_text().strip())
    manifest = json.loads((directory / "manifest.json").read_text())
    completed, missing_evidence = collect_ui_evidence(directory, manifest)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repository, text=True).strip()
    reviewed, visual_passed = collect_visual_review(directory, manifest)
    native_checks, missing_native_checks = collect_native_checks(directory, manifest)
    native_checks_complete = len(native_checks) == len(REQUIRED_NATIVE_CHECKS) and not missing_native_checks
    ui_complete = len(completed) == len(manifest["cases"]) and not missing_evidence
    missing_phases = [phase for phase in REQUIRED_PHASES if phase not in phases]
    deterministic_passed = not missing_phases and all(status == 0 for status in phases.values())
    summary = {"revision": revision, "run_id": manifest["run_id"], "phases": phases, "ui_cases": completed,
               "screenshots_reviewed": reviewed, "visual_passed": visual_passed,
               "ui_complete": ui_complete, "missing_evidence": missing_evidence, "missing_phases": missing_phases,
               "native_checks": native_checks, "native_checks_complete": native_checks_complete,
               "missing_native_checks": missing_native_checks,
               "deterministic_passed": deterministic_passed,
               "passed": deterministic_passed and ui_complete and visual_passed and native_checks_complete
                         and all(case["passed"] for case in completed + native_checks)}
    attempt_paths = list((directory / "attempts").glob("*"))
    unrecorded = sum(path.is_dir() and not (path / "result.json").is_file() for path in attempt_paths)
    attempts = [json.loads(path.read_text()) for path in (directory / "attempts").glob("*/result.json")]
    current_attempts = [attempt for attempt in attempts + completed if attempt.get("run_id") == manifest["run_id"]]
    summary["attempt_totals"] = {"recorded": len(current_attempts) + unrecorded, "unrecorded": unrecorded}
    for outcome in ("passed", "setup_blocked", "invalid_replay", "pending_setup", "pending_replay", "diagnostic", "failed"):
        summary["attempt_totals"][outcome] = sum(attempt_outcome(attempt) == outcome for attempt in current_attempts)
    (directory / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2))
    lines = ["# 编辑边界自动化测试报告", "", f"Git: `{revision}`", "", "| 阶段 | 退出码 |", "|---|---:|"]
    lines += [f"| {name} | {status} |" for name, status in phases.items()]
    source_passed = sum(attempt_outcome(case) == "passed" for case in completed)
    lines += ["", f"已记录 UI 用例：{len(completed)}/{len(manifest['cases'])}。有效重放中的源码检查通过：{source_passed}。",
              f"本轮全部尝试（含重试前证据）：{summary['attempt_totals']}。",
              f"截图逐项复核完成：{reviewed}；视觉验收通过：{visual_passed}；整体通过：{summary['passed']}。",
              f"必需原生专项：{len(native_checks)}/{len(REQUIRED_NATIVE_CHECKS)}；缺少：{', '.join(missing_native_checks) or '无'}。",
              f"缺少阶段：{', '.join(missing_phases) or '无'}。缺少窗口证据：{', '.join(missing_evidence) or '无'}。",
              "", "截图人工/模型复核另见 visual-review.md。",
              "", "回归阶段包含已知失败用例；非零退出码必须修复或明确记录，不能用 ignored 数量代替通过。"]
    (directory / "report.md").write_text("\n".join(lines) + "\n")
    return summary


def acceptance_status(summary):
    phase_failure = any(status != 0 and not (phase == "visual_metrics" and status == 2)
                        for phase, status in summary["phases"].items())
    source_failure = any(not case["passed"] and case.get("replayValid", True)
                         for case in summary["ui_cases"] + summary.get("native_checks", []))
    visual_failure = summary["screenshots_reviewed"] and not summary["visual_passed"]
    if phase_failure or source_failure or visual_failure:
        return 1
    return 0 if summary["passed"] else 2


def bundle(directory, repository):
    manifest_path = directory / "manifest.json"
    manifest = json.loads(manifest_path.read_text())
    run_identifier = uuid.UUID(manifest["run_id"])
    build_identifier = uuid.uuid4()
    display_name = f"textora-audit-{build_identifier.hex[:8]}"
    app = directory / "bundles" / str(run_identifier) / str(build_identifier) / "textora-editing-audit.app"
    bundle_identifier = f"com.dan.textora.editing-audit.run-{run_identifier.hex}.build-{build_identifier.hex}"
    executable = app / "Contents" / "MacOS" / "textora"
    executable.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(repository / "target" / "debug" / "textora", executable)
    with (app / "Contents" / "Info.plist").open("wb") as stream:
        plistlib.dump({"CFBundleExecutable": "textora", "CFBundleIdentifier": bundle_identifier,
                      "CFBundleName": display_name, "CFBundleDisplayName": display_name,
                      "CFBundlePackageType": "APPL"}, stream)
    manifest["bundle"] = {"path": str(app), "identifier": bundle_identifier, "display_name": display_name,
                          "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest()}
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2))
    print(f"Computer Use app: {app}")


def fixture_setup_matches(result):
    if "setupMatches" in result:
        return result["setupMatches"]
    checkpoints = result.get("checkpoints", [])
    return bool(checkpoints) and checkpoints[0].get("sourceMatches", False)


def contacts(directory):
    from PIL import Image, ImageDraw
    manifest = json.loads((directory / "manifest.json").read_text())
    cases = sorted(path.parent for path in (directory / "screenshots").glob("*/result.json")
                   if json.loads(path.read_text()).get("run_id") == manifest["run_id"])
    thumbnail_width, thumbnail_height, label_height = 360, 180, 28
    rows_per_page = 8
    for page_start in range(0, len(cases), rows_per_page):
        batch = cases[page_start:page_start + rows_per_page]
        sheet = Image.new("RGB", (thumbnail_width * 4, (thumbnail_height + label_height) * len(batch)), "white")
        drawing = ImageDraw.Draw(sheet)
        for row, case in enumerate(batch):
            for column, stage in enumerate(["before", "after", "undo", "redo"]):
                stage_path = case / f"{stage}.png"
                result = json.loads((case / "result.json").read_text())
                if not stage_path.exists() or (not fixture_setup_matches(result) and stage != "before"):
                    drawing.text((column * thumbnail_width + 4, row * (thumbnail_height + label_height) + 4),
                                 f"{case.name}: {stage} missing or skipped", fill="red")
                    continue
                with Image.open(stage_path) as screenshot:
                    # Editor canvas in the native 2x screenshot, excluding sidebar/titlebar.
                    crop = screenshot.crop((440, 72, min(screenshot.width, 1440), min(screenshot.height, 520)))
                    crop.thumbnail((thumbnail_width, thumbnail_height))
                    y = row * (thumbnail_height + label_height)
                    sheet.paste(crop, (column * thumbnail_width, y + label_height))
                    drawing.text((column * thumbnail_width + 4, y + 4), f"{case.name}: {stage}", fill="black")
        sheet.save(directory / f"contact-{page_start // rows_per_page + 1:02}.png")
    fixtures = {case["id"]: case for case in manifest["cases"]}
    for captures in cases:
        write_intermediate_contact_sheet(directory, captures, fixtures[captures.name])


def write_intermediate_contact_sheet(directory, captures, fixture):
    from PIL import Image, ImageDraw
    thumbnail_width, thumbnail_height, label_height = 360, 180, 28
    stages = ["position-source", "position-markdown", "before"]
    stages += [f"step-{index:02}" for index in range(1, len(fixture["keys"]) + 1)]
    stages += ["after", "undo", "redo", "reopen"]
    columns = 4
    cell_height = thumbnail_height + label_height
    row_count = (len(stages) + columns - 1) // columns
    sheet = Image.new("RGB", (thumbnail_width * columns, cell_height * row_count), "white")
    drawing = ImageDraw.Draw(sheet)
    for index, stage in enumerate(stages):
        x = index % columns * thumbnail_width
        y = index // columns * cell_height
        drawing.text((x + 4, y + 4), stage, fill="black")
        screenshot_path = captures / f"{stage}.png"
        if not screenshot_path.exists():
            drawing.text((x + 4, y + label_height), "Missing evidence", fill="red")
            continue
        with Image.open(screenshot_path) as screenshot:
            crop = screenshot.crop((440, 72, min(screenshot.width, 1440), min(screenshot.height, 600)))
            crop.thumbnail((thumbnail_width, thumbnail_height))
            sheet.paste(crop, (x, y + label_height))
    sheet.save(directory / f"intermediate-{captures.name}.png")




def image_check_ready(captures, case, result, manifest):
    if result.get("run_id") != manifest["run_id"] or result.get("id") != case["id"]:
        return False
    if result.get("replayValid") is False or result.get("setupMatches") is False or result.get("saveEachStep"):
        return False
    if manifest.get("bundle") and result.get("build") != manifest["bundle"]:
        return False
    stages = ["before", "after"] + [f"step-{index:02}" for index in range(1, len(case["keys"]) + 1)]
    return all((captures / f"{stage}.png").is_file() for stage in stages)


def is_key_batch(action):
    return isinstance(action, dict) and "keys" in action


def layout_metrics_passed(case, steps, restored):
    if not restored or len(steps) != len(case["keys"]):
        return False
    return all(step["layout_changed"] or is_key_batch(action)
               for action, step in zip(case["keys"], steps))


def image_metrics(directory):
    from PIL import Image
    import numpy as np

    color_distance = 10
    row_pixel_delta = 10
    minimum_changed_rows = 3

    def row_signature(image_path):
        with Image.open(image_path) as image:
            if image.width < 1440 or image.height < 600:
                raise ValueError("Screenshot dimensions require editor-canvas crop recalibration")
            canvas = image.convert("RGB").crop((440, 72, 1440, 600))
            background = max(canvas.getcolors(canvas.width * canvas.height), key=lambda entry: entry[0])[1]
            pixels = np.asarray(canvas, dtype=np.int16)
            foreground = np.max(np.abs(pixels - np.asarray(background)), axis=2) > color_distance
            caret = (pixels[:, :, 0] - pixels[:, :, 1] > 30) & (pixels[:, :, 1] - pixels[:, :, 2] > 15)
            return (foreground & ~caret).sum(axis=1)

    def changed_rows(first, second):
        return int((np.abs(first - second) > row_pixel_delta).sum())

    manifest = json.loads((directory / "manifest.json").read_text())
    completed = []
    missing = []
    for case in manifest["cases"]:
        if "enter-" not in case["id"]:
            continue
        captures = directory / "screenshots" / case["id"]
        result_path = captures / "result.json"
        if not result_path.exists():
            missing.append(case["id"])
            continue
        result = json.loads(result_path.read_text())
        if not image_check_ready(captures, case, result, manifest):
            missing.append(case["id"])
            continue
        previous = row_signature(captures / "before.png")
        before = previous
        steps = []
        for index, key in enumerate(case["keys"], start=1):
            current = row_signature(captures / f"step-{index:02}.png")
            difference = changed_rows(previous, current)
            steps.append({"key": key, "changed_content_rows": difference,
                          "layout_changed": difference >= minimum_changed_rows,
                          "capture_scope": "batch-final" if is_key_batch(key) else "single-key"})
            previous = current
        restored = changed_rows(before, row_signature(captures / "after.png")) < minimum_changed_rows
        completed.append({"id": case["id"], "steps": steps, "layout_restored": restored,
                          "passed": layout_metrics_passed(case, steps, restored)})
    summary = {"run_id": manifest["run_id"], "cases": completed, "missing": missing,
               "note": "Calibrated for the observed macOS 2x canvas; batches verify final restoration, individual keys also require a layout change. Screenshot review remains required; these are not pixel-perfect golden baselines."}
    (directory / "visual-metrics.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2))
    status = 1 if any(not case["passed"] for case in completed) else (2 if missing else 0)
    (directory / "visual_metrics.status").write_text(f"{status}\n")
    print(f"Image metrics: {len(completed)} cases, {sum(not case['passed'] for case in completed)} require review, {len(missing)} pending")
    return status

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=["prepare", "summarize", "bundle", "contacts", "images", "fingerprints"])
    parser.add_argument("directory", type=Path)
    arguments = parser.parse_args()
    repository = Path(__file__).resolve().parent.parent
    directory = arguments.directory.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    if arguments.action == "prepare":
        prepare(directory, repository)
    elif arguments.action == "bundle":
        bundle(directory, repository)
    elif arguments.action == "contacts":
        contacts(directory)
    elif arguments.action == "images":
        raise SystemExit(image_metrics(directory))
    elif arguments.action == "fingerprints":
        manifest = json.loads((directory / "manifest.json").read_text())
        completed, _ = collect_ui_evidence(directory, manifest)
        fingerprints = {case["id"]: case_fingerprint(directory / "screenshots" / case["id"])
                        for case in completed}
        fingerprints.update({identifier: case_fingerprint(directory / "native-checks" / identifier)
                             for identifier in REQUIRED_NATIVE_CHECKS
                             if (directory / "native-checks" / identifier).is_dir()})
        print(json.dumps(fingerprints, indent=2))
    else:
        raise SystemExit(acceptance_status(summarize(directory, repository)))


if __name__ == "__main__":
    main()
