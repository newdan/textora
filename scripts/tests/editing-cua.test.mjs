import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { beginEditingCase, openEditingCase, positionEditingCase, startEditingPosition, advanceEditingPosition, finishEditingPosition, replayEditingActions, startEditingHistory, advanceEditingHistory, replayEditingHistory, finishEditingCase, verifyEditingHistory, runEditingCase } from "../editing-cua.mjs";

// Reproduce the observed dialog race without sending input to the computer.
class DelayedFileDialog {
  constructor(dialogObservations) {
    this.dialogObservations = dialogObservations;
    this.observationCount = 0;
    this.surface = "editor";
    this.fixturePath = "";
    this.positioningObservations = [];
  }

  async getAXState() {
    if (this.surface === "goto") return "2 text field Path, ID: PathTextField";
    if (this.surface === "panel") return "3 button Open, ID: OKButton";
    if (this.surface === "opening") {
      this.observationCount++;
      if (this.observationCount <= this.dialogObservations) {
        return `1 window ${path.basename(this.fixturePath)}\n3 button Open, ID: OKButton`;
      }
      this.surface = "editor";
    }
    return `1 window ${path.basename(this.fixturePath) || "textora"}`;
  }

  async getAXStateAndScreenshot() {
    return { state: await this.getAXState(), screenshot: Buffer.from("synthetic test screenshot") };
  }

  async pressKey(key) {
    this.events ??= [];
    this.events.push(key);
    this.pressedKeys ??= [];
    this.pressedKeys.push(key);
    if (key === "super+w") this.fixturePath = "";
    if (key === "super+o") this.surface = "panel";
    if (key === "super+shift+g") this.surface = "goto";
    if (key === "Return" && this.surface === "goto") this.surface = "panel";
    if (key === "super+shift+m") this.positioningObservations.push(this.observationCount);
  }

  async setValue(_, fixturePath) {
    this.fixturePath = fixturePath;
  }

  async click(target) {
    this.events ??= [];
    if (Array.isArray(target)) {
      this.events.push("source-focus");
      return;
    }
    this.surface = "opening";
  }
}

async function fixtureManifest(context) {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "textora-cua-harness-"));
  context.after(() => fs.rm(directory, { recursive: true, force: true }));
  const manifestPath = path.join(directory, "manifest.json");
  await fs.writeFile(manifestPath, JSON.stringify({ run_id: "harness-test", cases: [{
    id: "paragraph-paragraph-enter-1", source: "before\n\nafter", cursor_prefix: "before",
    keys: ["Return", "BackSpace"], expected_source: "before\n\nafter", expectation: "fixture for dialog readiness test"
  }] }));
  return manifestPath;
}

test("positioning waits for the file dialog to disappear", async context => {
  const app = new DelayedFileDialog(3);
  await runEditingCase(app, await fixtureManifest(context), "paragraph-paragraph-enter-1");
  assert.deepEqual(app.positioningObservations, [4, 4]);
});

test("a filename inside a remaining dialog does not count as an opened editor", async context => {
  const app = new DelayedFileDialog(Infinity);
  const manifestPath = await fixtureManifest(context);
  await assert.rejects(runEditingCase(app, manifestPath, "paragraph-paragraph-enter-1"),
    /fixture window is not ready/);
  assert.deepEqual(app.positioningObservations, []);
  const recorded = JSON.parse(await fs.readFile(path.join(path.dirname(manifestPath), "screenshots", "paragraph-paragraph-enter-1", "result.json"), "utf8"));
  assert.equal(recorded.replayValid, false);
  assert.equal(recorded.passed, false);
});

test("batched fast keys retain enough undo and redo actions", async context => {
  const app = new DelayedFileDialog(0);
  const manifestPath = await fixtureManifest(context);
  const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
  manifest.cases[0].keys = [{ keys: ["Return", "Return", "Return", "BackSpace", "BackSpace", "BackSpace"] }];
  await fs.writeFile(manifestPath, JSON.stringify(manifest));
  await runEditingCase(app, manifestPath, "paragraph-paragraph-enter-1");
  assert.equal(app.pressedKeys.filter(key => key === "super+z").length, 6);
  assert.equal(app.pressedKeys.filter(key => key === "super+shift+z").length, 6);
});

test("retry filenames cannot match the title of an earlier attempt", async context => {
  const app = new DelayedFileDialog(0);
  const manifestPath = await fixtureManifest(context);
  const firstAttempt = await runEditingCase(app, manifestPath, "paragraph-paragraph-enter-1");
  const secondAttempt = await runEditingCase(app, manifestPath, "paragraph-paragraph-enter-1");
  assert.notEqual(path.basename(secondAttempt.replayPath), path.basename(firstAttempt.replayPath));
});

test("source positioning acquires focus after switching modes", async context => {
  const app = new DelayedFileDialog(0);
  await runEditingCase(app, await fixtureManifest(context), "paragraph-paragraph-enter-1", {sourceFocusPoint:[600,100]});
  const toggle = app.events.indexOf("super+shift+m");
  const focus = app.events.indexOf("source-focus");
  const documentStart = app.events.indexOf("super+Up");
  assert.ok(toggle < focus && focus < documentStart, "focus must belong to the source view before navigation");
});

test("preparing an attempt records evidence before the caller starts replay", async context => {
  const app = new DelayedFileDialog(0);
  const manifestPath = await fixtureManifest(context);
  const evidencePath = path.join(path.dirname(manifestPath), "screenshots", "paragraph-paragraph-enter-1", "result.json");
  const observe = app.getAXStateAndScreenshot.bind(app);
  let initialEvidence;
  app.getAXStateAndScreenshot = async () => {
    initialEvidence ??= JSON.parse(await fs.readFile(evidencePath, "utf8"));
    return observe();
  };
  await beginEditingCase(app, manifestPath, "paragraph-paragraph-enter-1");
  assert.equal(initialEvidence.replayStage, "preparing");
  const recorded = JSON.parse(await fs.readFile(path.join(path.dirname(manifestPath), "screenshots", "paragraph-paragraph-enter-1", "result.json"), "utf8"));
  assert.equal(recorded.replayStage, "prepared");
  assert.equal(recorded.passed, false);
  assert.equal(recorded.replayValid, false);
});

test("editing can pause before history without replaying input or accepting partial evidence", async context => {
  const app = new DelayedFileDialog(0);
  const manifestPath = await fixtureManifest(context);
  const prepared = await beginEditingCase(app, manifestPath, "paragraph-paragraph-enter-1");
  await replayEditingActions(app, prepared);
  const evidencePath = path.join(prepared.directory, "result.json");
  const partial = JSON.parse(await fs.readFile(evidencePath, "utf8"));
  assert.equal(partial.passed, false);
  assert.equal(partial.nextPhase, "history");
  assert.equal(app.pressedKeys.filter(key => key === "super+z").length, 0);
  const inputCount = app.pressedKeys.filter(key => key === "BackSpace").length;
  const result = await verifyEditingHistory(app, prepared);
  assert.equal(result.passed, true);
  assert.equal(app.pressedKeys.filter(key => key === "BackSpace").length, inputCount);
  assert.deepEqual(result.checkpoints.map(checkpoint => checkpoint.stage), ["before", "after", "undo", "redo"]);
});

test("history verification stays pending until the reopen phase finishes", async context => {
  const app = new DelayedFileDialog(0);
  const prepared = await beginEditingCase(app, await fixtureManifest(context), "paragraph-paragraph-enter-1");
  await replayEditingActions(app, prepared);
  await replayEditingHistory(app, prepared);
  const partial = JSON.parse(await fs.readFile(path.join(prepared.directory, "result.json"), "utf8"));
  assert.equal(partial.passed, false);
  assert.equal(partial.nextPhase, "reopen");
  assert.equal((await finishEditingCase(app, prepared)).passed, true);
});

test("each history key synchronizes with the native window before the next key", async context => {
  const app = new DelayedFileDialog(0);
  const pressKey = app.pressKey.bind(app);
  const observe = app.getAXState.bind(app);
  let pendingHistoryKey = false;
  app.pressKey = async key => {
    if (key === "super+z" || key === "super+shift+z") {
      assert.equal(pendingHistoryKey, false, "consecutive native history keys need a fresh observation");
      pendingHistoryKey = true;
    }
    await pressKey(key);
  };
  app.getAXState = async () => {
    pendingHistoryKey = false;
    return observe();
  };
  await runEditingCase(app, await fixtureManifest(context), "paragraph-paragraph-enter-1");
});

test("source positioning uses the nearer document edge and counts CRLF and emoji as graphemes", async context => {
  const app = new DelayedFileDialog(0);
  const manifestPath = await fixtureManifest(context);
  const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
  manifest.cases[0].source = "long first paragraph\r\n👩🏽‍💻x";
  manifest.cases[0].cursor_prefix = "long first paragraph";
  await fs.writeFile(manifestPath, JSON.stringify(manifest));
  await beginEditingCase(app, manifestPath, "paragraph-paragraph-enter-1");
  assert.ok(app.pressedKeys.includes("super+Down"));
  assert.equal(app.pressedKeys.filter(key => key === "Left").length, 3);
  assert.equal(app.pressedKeys.filter(key => key === "Right").length, 0);
});

test("source mode can use an observed button instead of an input-method-dependent shortcut", async context => {
  const app = new DelayedFileDialog(0);
  await beginEditingCase(app, await fixtureManifest(context), "paragraph-paragraph-enter-1",
    { sourceTogglePoint: [1978, 32], sourceFocusPoint: [600, 100] });
  assert.equal(app.pressedKeys.filter(key => key === "super+shift+m").length, 0);
  assert.equal(app.events.filter(event => event === "source-focus").length, 3);
});

test("opening can yield before source positioning and keeps incomplete evidence pending", async context => {
  const app = new DelayedFileDialog(0);
  const prepared = await openEditingCase(app, await fixtureManifest(context), "paragraph-paragraph-enter-1");
  assert.deepEqual(app.positioningObservations, []);
  const partial = JSON.parse(await fs.readFile(path.join(prepared.directory, "result.json"), "utf8"));
  assert.equal(partial.passed, false);
  assert.equal(partial.nextPhase, "position");
  await positionEditingCase(app, prepared);
  assert.equal(prepared.checkpoints[0].sourceMatches, true);
});

test("positioning yields after a bounded key batch and cannot finish with navigation pending", async context => {
  const app = new DelayedFileDialog(0);
  const prepared = await openEditingCase(app, await fixtureManifest(context), "paragraph-paragraph-enter-1");
  await startEditingPosition(app, prepared);
  await advanceEditingPosition(app, prepared);
  assert.equal(prepared.positioning.remaining, 2);
  assert.equal(prepared.checkpoints.length, 0);
  await assert.rejects(finishEditingPosition(app, prepared), /navigation is still pending/);
  await advanceEditingPosition(app, prepared);
  await finishEditingPosition(app, prepared);
  assert.equal(prepared.checkpoints[0].sourceMatches, true);
});

test("editing rejects incomplete positioning and duplicate replay before sending keys", async context => {
  const app = new DelayedFileDialog(0);
  const prepared = await openEditingCase(app, await fixtureManifest(context), "paragraph-paragraph-enter-1");
  const beforePositioning = app.pressedKeys.length;
  await assert.rejects(replayEditingActions(app, prepared), /positioning must complete/);
  assert.equal(app.pressedKeys.length, beforePositioning);
  await positionEditingCase(app, prepared);
  await replayEditingActions(app, prepared);
  const afterReplay = app.pressedKeys.length;
  await assert.rejects(replayEditingActions(app, prepared), /already been replayed/);
  assert.equal(app.pressedKeys.length, afterReplay);
});

test("history advances in bounded batches and cannot finish before all redo keys", async context => {
  const app = new DelayedFileDialog(0);
  const manifestPath = await fixtureManifest(context);
  const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
  manifest.cases[0].keys = ["Return", "Return", "Return", "BackSpace", "BackSpace", "BackSpace"];
  await fs.writeFile(manifestPath, JSON.stringify(manifest));
  const prepared = await beginEditingCase(app, manifestPath, "paragraph-paragraph-enter-1");
  await replayEditingActions(app, prepared);
  await startEditingHistory(app, prepared);
  await advanceEditingHistory(app, prepared);
  assert.equal(prepared.historyProgress.phase, "undo");
  assert.equal(prepared.historyProgress.remaining, 2);
  assert.equal(prepared.checkpoints.length, 2);
  await assert.rejects(finishEditingCase(app, prepared), /History verification must complete/);
  while (prepared.historyProgress.phase !== "completed") await advanceEditingHistory(app, prepared);
  assert.equal((await finishEditingCase(app, prepared)).passed, true);
  assert.equal(app.pressedKeys.filter(key => key === "super+z").length, 6);
  assert.equal(app.pressedKeys.filter(key => key === "super+shift+z").length, 6);
});

test("completed reopen verification closes its saved disposable fixture", async context => {
  const app = new DelayedFileDialog(0);
  const manifestPath = await fixtureManifest(context);
  const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
  manifest.cases[0].reopen_after = true;
  await fs.writeFile(manifestPath, JSON.stringify(manifest));
  const outcome = await runEditingCase(app, manifestPath, "paragraph-paragraph-enter-1");
  assert.equal(outcome.passed, true);
  assert.equal(outcome.fixtureClosed, true);
  assert.equal(app.pressedKeys.filter(key => key === "super+w").length, 2);
  assert.equal(outcome.checkpoints.at(-1).stage, "reopen");
});
