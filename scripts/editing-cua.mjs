/** Native macOS replay. Run inside cua_repl; no shell/OS input injection. */
import fs from "node:fs/promises";
import path from "node:path";
import { randomUUID } from "node:crypto";

const WINDOW_READY_OBSERVATIONS = 8;
const NATIVE_KEY_BATCH_LIMIT = 4;
const FILE_DIALOG_CONTROL = /ID: (?:PathTextField|OKButton)/;

async function observe(app) {
  return app.getAXState({ emit: false, disableDiffing: true });
}

export async function capture(app, directory, stage) {
  const snapshot = await app.getAXStateAndScreenshot({ emit: false, disableDiffing: true });
  if (!snapshot.screenshot) throw new Error("Native screenshot is unavailable");
  if (FILE_DIALOG_CONTROL.test(snapshot.state)) throw new Error("A file dialog is still covering the editor");
  await fs.writeFile(path.join(directory, `${stage}.png`), snapshot.screenshot);
  await fs.writeFile(path.join(directory, `${stage}.ax.txt`), snapshot.state);
}

async function waitForFixtureWindow(app, fixturePath) {
  const filename = path.basename(fixturePath);
  for (let observation = 0; observation < WINDOW_READY_OBSERVATIONS; observation++) {
    const state = await observe(app);
    const fixtureWindow = state.split("\n").some(line => /\bwindow\b/i.test(line) && line.includes(filename));
    if (fixtureWindow && !FILE_DIALOG_CONTROL.test(state)) return;
  }
  throw new Error("Test fixture window is not ready after the file dialog action");
}

export async function openFixture(app, fixturePath) {
  await observe(app);
  await app.pressKey("super+o");
  await observe(app);
  await app.pressKey("super+shift+g");
  const pathSheet = await observe(app);
  const pathField = pathSheet.match(/(?:^|\n)[^\n]*?\b(\d+) text field[^\n]*ID: PathTextField/);
  if (!pathField) throw new Error("Go To path field is not visible");
  await app.setValue(Number(pathField[1]), fixturePath);
  await observe(app);
  await app.pressKey("Return");
  const filePanel = await observe(app);
  const openButton = filePanel.match(/(?:^|\n)[^\n]*?\b(\d+) button Open, ID: OKButton/);
  if (!openButton) throw new Error("Selected test fixture cannot be opened");
  await app.click(Number(openButton[1]));
  await waitForFixtureWindow(app, fixturePath);
}

async function toggleSourceMode(app, sourceTogglePoint) {
  if (sourceTogglePoint) {
    await app.getAXStateAndScreenshot({ emit: false });
    await app.click(sourceTogglePoint);
  } else {
    await app.pressKey("super+shift+m");
  }
  await observe(app);
}

function sourceNavigationPlan(source, prefix) {
  const segmenter = new Intl.Segmenter("und", { granularity: "grapheme" });
  const graphemes = [...segmenter.segment(source)];
  const targetIndex = prefix.length === source.length ? graphemes.length
    : graphemes.findIndex(grapheme => grapheme.index === prefix.length);
  if (!source.startsWith(prefix) || targetIndex < 0) {
    throw new Error("Fixture cursor must be at a source grapheme boundary");
  }
  const fromEnd = graphemes.length - targetIndex < targetIndex;
  return { edge: fromEnd ? "super+Down" : "super+Up", chord: fromEnd ? "Left" : "Right",
    remaining: fromEnd ? graphemes.length - targetIndex : targetIndex };
}

async function runSetupPhase(app, prepared, action) {
  try {
    return await action();
  } catch (error) {
    await recordReplayFailure(app, prepared.directory, prepared.manifest, prepared.fixture, false, error);
    throw error;
  }
}

export async function startEditingPosition(app, prepared) {
  return runSetupPhase(app, prepared, async () => {
    const { fixture, options } = prepared;
    await waitForFixtureWindow(app, fixture.path);
    if (options.editorFocusPoint) await focusEditor(app, options.editorFocusPoint);
    prepared.positioning = sourceNavigationPlan(fixture.source, fixture.cursor_prefix);
    await toggleSourceMode(app, options.sourceTogglePoint);
    if (options.sourceFocusPoint) await focusEditor(app, options.sourceFocusPoint);
    await app.pressKey(prepared.positioning.edge);
    await observe(app);
  });
}

export async function advanceEditingPosition(app, prepared) {
  return runSetupPhase(app, prepared, async () => {
    const count = Math.min(NATIVE_KEY_BATCH_LIMIT, prepared.positioning.remaining);
    for (let index = 0; index < count; index++) {
      await app.pressKey(prepared.positioning.chord);
      prepared.positioning.remaining--;
    }
    await observe(app);
  });
}

async function checkpoint(app, fixturePath, directory, stage, expectedSource) {
  await app.pressKey("super+s");
  await capture(app, directory, stage);
  const source = await fs.readFile(fixturePath, "utf8");
  await fs.writeFile(path.join(directory, `${stage}.md`), source);
  return { stage, source, expectedSource, sourceMatches: source === expectedSource };
}

async function preservePreviousAttempt(runDirectory, directory, caseId) {
  try {
    await fs.access(directory);
  } catch (error) {
    if (error.code === "ENOENT") return;
    throw error;
  }
  const attempts = path.join(runDirectory, "attempts");
  await fs.mkdir(attempts, { recursive: true });
  await fs.rename(directory, path.join(attempts, `${caseId}-${randomUUID()}`));
}

async function focusEditor(app, point) {
  await app.getAXStateAndScreenshot({ emit: false });
  await app.click(point);
  await observe(app);
}

async function replayKeys(app, fixture, directory, saveEachStep) {
  const stepSources = [];
  for (const [index, key] of fixture.keys.entries()) {
    if (typeof key === "string") await app.pressKey(key);
    else if (Array.isArray(key.keys)) {
      for (const chord of key.keys) await app.pressKey(chord);
    }
    else if (typeof key.paste === "string") await app.paste(key.paste, { format: "text" });
    else if (typeof key.text === "string") await app.typeText(key.text);
    else throw new Error("Unsupported generated editing action");
    if (saveEachStep) await app.pressKey("super+s");
    const stage = `step-${String(index + 1).padStart(2, "0")}`;
    await capture(app, directory, stage);
    if (saveEachStep) {
      const source = await fs.readFile(fixture.path, "utf8");
      await fs.writeFile(path.join(directory, `${stage}.md`), source);
      stepSources.push({ stage, key, source });
    }
  }
  return stepSources;
}

export async function runEditingCase(app, manifestPath, caseId, options = {}) {
  return completeEditingCase(app, await beginEditingCase(app, manifestPath, caseId, options));
}

export async function beginEditingCase(app, manifestPath, caseId, options = {}) {
  return positionEditingCase(app, await openEditingCase(app, manifestPath, caseId, options));
}

export async function openEditingCase(app, manifestPath, caseId, options = {}) {
  const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
  const selected = manifest.cases.find(candidate => candidate.id === caseId);
  if (!selected) throw new Error(`Unknown editing case: ${caseId}`);
  if (!/^[a-z0-9-]+$/.test(caseId)) throw new Error("Invalid generated case identifier");
  const runDirectory = path.dirname(manifestPath);
  const replayIdentifier = randomUUID();
  const replayDirectory = path.join(runDirectory, "replays", manifest.run_id, replayIdentifier);
  await fs.mkdir(replayDirectory, { recursive: true });
  const fixture = { ...selected, path: path.join(replayDirectory, `${caseId}-${replayIdentifier}.md`) };
  const directory = path.join(runDirectory, "screenshots", caseId);
  await preservePreviousAttempt(runDirectory, directory, caseId);
  await fs.mkdir(directory, { recursive: true });
  // Fixtures are disposable outputs created by editing-audit.py, never user documents.
  await fs.writeFile(fixture.path, fixture.source);
  const initialEvidence = { run_id: manifest.run_id, id: caseId, build: manifest.bundle,
    replayPath: fixture.path, replayStage: "preparing", passed: false, replayValid: false, checkpoints: [] };
  await fs.writeFile(path.join(directory, "result.json"), JSON.stringify(initialEvidence, null, 2));
  try {
    await openFixture(app, fixture.path);
    await fs.writeFile(path.join(directory, "result.json"), JSON.stringify({
      ...initialEvidence, nextPhase: "position"
    }, null, 2));
    return { manifest, fixture, directory, options, checkpoints: [] };
  } catch (error) {
    await recordReplayFailure(app, directory, manifest, fixture, false, error);
    throw error;
  }
}

export async function positionEditingCase(app, prepared) {
  await startEditingPosition(app, prepared);
  while (prepared.positioning.remaining > 0) await advanceEditingPosition(app, prepared);
  return finishEditingPosition(app, prepared);
}

export async function finishEditingPosition(app, prepared) {
  if (prepared.positioning?.remaining !== 0) throw new Error("Source navigation is still pending");
  return runSetupPhase(app, prepared, async () => {
    const { manifest, fixture, directory, options, checkpoints } = prepared;
    await capture(app, directory, "position-source");
    await toggleSourceMode(app, options.sourceTogglePoint);
    await capture(app, directory, "position-markdown");
    if (fixture.select_all) {
      await app.pressKey("super+a");
      await observe(app);
    }
    checkpoints.push(await checkpoint(app, fixture.path, directory, "before", fixture.source));
    if (!checkpoints[0].sourceMatches) {
      prepared.blockedResult = { run_id: manifest.run_id, id: fixture.id, build: manifest.bundle,
        passed: false, setupMatches: false, replayPath: fixture.path, checkpoints, visualReview: "pending",
        error: "Opening/positioning/saving changed the fixture before the editing action" };
      await fs.writeFile(path.join(directory, "result.json"), JSON.stringify(prepared.blockedResult, null, 2));
      return prepared;
    }
    const preparedEvidence = { run_id: manifest.run_id, id: fixture.id, build: manifest.bundle,
      replayPath: fixture.path, setupMatches: true, passed: false, replayValid: false,
      replayStage: "prepared", sourceFocusPoint: options.sourceFocusPoint,
      sourceTogglePoint: options.sourceTogglePoint, checkpoints };
    await fs.writeFile(path.join(directory, "result.json"), JSON.stringify(preparedEvidence, null, 2));
    return prepared;
  });
}

export async function completeEditingCase(app, prepared) {
  await replayEditingActions(app, prepared);
  return verifyEditingHistory(app, prepared);
}

export async function replayEditingActions(app, prepared) {
  if (prepared.blockedResult) return;
  const { manifest, fixture, directory, options, checkpoints } = prepared;
  if (!checkpoints.some(checkpoint => checkpoint.stage === "before")) {
    throw new Error("Fixture positioning must complete before editing");
  }
  if (checkpoints.some(checkpoint => checkpoint.stage === "after")) {
    throw new Error("Editing actions have already been replayed");
  }
  try {
    await waitForFixtureWindow(app, fixture.path);
    prepared.stepSources = await replayKeys(app, fixture, directory, options.saveEachStep);
    checkpoints.push(await checkpoint(app, fixture.path, directory, "after", fixture.expected_source));
    const partial = { run_id: manifest.run_id, id: fixture.id, build: manifest.bundle,
      replayPath: fixture.path, setupMatches: true, passed: false, replayValid: false,
      replayStage: "prepared", nextPhase: "history", checkpoints };
    await fs.writeFile(path.join(directory, "result.json"), JSON.stringify(partial, null, 2));
  } catch (error) {
    await recordReplayFailure(app, directory, manifest, fixture, true, error);
    throw error;
  }
}

export async function verifyEditingHistory(app, prepared) {
  await replayEditingHistory(app, prepared);
  return finishEditingCase(app, prepared);
}

export async function replayEditingHistory(app, prepared) {
  if (prepared.blockedResult) return;
  await startEditingHistory(app, prepared);
  while (prepared.historyProgress.phase !== "completed") await advanceEditingHistory(app, prepared);
}

export async function startEditingHistory(app, prepared) {
  if (!prepared.checkpoints.some(checkpoint => checkpoint.stage === "after")) {
    throw new Error("Editing actions must complete before history verification");
  }
  if (prepared.historyProgress) throw new Error("History verification has already started");
  // Navigation, consumed keys and coalescing can create fewer history entries.
  const budget = prepared.fixture.keys.reduce((count, action) => count + (action.keys?.length ?? 1), 0);
  prepared.historyProgress = { phase: "undo", remaining: budget, budget };
  await waitForFixtureWindow(app, prepared.fixture.path);
}

export async function advanceEditingHistory(app, prepared) {
  const { manifest, fixture, directory, checkpoints, historyProgress } = prepared;
  if (historyProgress.phase === "completed") return;
  try {
    const chord = historyProgress.phase === "undo" ? "super+z" : "super+shift+z";
    const count = Math.min(NATIVE_KEY_BATCH_LIMIT, historyProgress.remaining);
    for (let index = 0; index < count; index++) {
      await app.pressKey(chord);
      historyProgress.remaining--;
      await observe(app);
    }
    if (historyProgress.remaining === 0) {
      const stage = historyProgress.phase;
      const expected = stage === "undo" ? fixture.source
        : checkpoints.find(checkpoint => checkpoint.stage === "after").source;
      checkpoints.push(await checkpoint(app, fixture.path, directory, stage, expected));
      historyProgress.phase = stage === "undo" ? "redo" : "completed";
      historyProgress.remaining = stage === "undo" ? historyProgress.budget : 0;
    }
    const partial = { run_id: manifest.run_id, id: fixture.id, build: manifest.bundle,
      replayPath: fixture.path, setupMatches: true, passed: false, replayValid: false,
      replayStage: "prepared", nextPhase: historyProgress.phase === "completed" ? "reopen" : "history",
      historyProgress, checkpoints };
    await fs.writeFile(path.join(directory, "result.json"), JSON.stringify(partial, null, 2));
  } catch (error) {
    await recordReplayFailure(app, directory, manifest, fixture, true, error);
    throw error;
  }
}

export async function finishEditingCase(app, prepared) {
  if (prepared.blockedResult) return prepared.blockedResult;
  const { manifest, fixture, directory, options, checkpoints, stepSources } = prepared;
  if (!checkpoints.some(checkpoint => checkpoint.stage === "redo")) {
    throw new Error("History verification must complete before reopening");
  }
  try {
    await waitForFixtureWindow(app, fixture.path);
    if (fixture.reopen_after) {
      await app.pressKey("super+w");
      await observe(app);
      await openFixture(app, fixture.path);
      checkpoints.push(await checkpoint(app, fixture.path, directory, "reopen", fixture.expected_source));
    }
    const passed = checkpoints.every(checkpoint => checkpoint.sourceMatches);
    let fixtureClosed = false;
    if (passed) {
      await waitForFixtureWindow(app, fixture.path);
      await app.pressKey("super+w");
      const closedState = await observe(app);
      if (closedState.split("\n").some(line => /\bwindow\b/i.test(line) && line.includes(path.basename(fixture.path)))) {
        throw new Error("Saved disposable fixture did not close after verification");
      }
      fixtureClosed = true;
    }
    const result = { run_id: manifest.run_id, id: fixture.id, build: manifest.bundle, setupMatches: true, fixtureClosed, editorFocusPoint: options.editorFocusPoint, sourceFocusPoint: options.sourceFocusPoint, sourceTogglePoint: options.sourceTogglePoint, replayPath: fixture.path, expectation: fixture.expectation, checkpoints, stepSources, saveEachStep: Boolean(options.saveEachStep),
                     passed, replayStage: "completed", visualReview: "pending" };
    await fs.writeFile(path.join(directory, "result.json"), JSON.stringify(result, null, 2));
    return result;
  } catch (error) {
    await recordReplayFailure(app, directory, manifest, fixture, true, error);
    throw error;
  }
}

async function recordReplayFailure(app, directory, manifest, fixture, setupMatches, error) {
  const result = { run_id: manifest.run_id, id: fixture.id, build: manifest.bundle,
    replayPath: fixture.path, passed: false, setupMatches, replayValid: false, replayStage: "failed",
    checkpoints: [], error: String(error.message ?? error) };
  try {
    const snapshot = await app.getAXStateAndScreenshot({ emit: false, disableDiffing: true });
    await fs.writeFile(path.join(directory, "error.ax.txt"), snapshot.state);
    if (snapshot.screenshot) await fs.writeFile(path.join(directory, "error.png"), snapshot.screenshot);
  } catch (captureError) {
    result.captureError = String(captureError.message ?? captureError);
  }
  await fs.writeFile(path.join(directory, "result.json"), JSON.stringify(result, null, 2));
}
