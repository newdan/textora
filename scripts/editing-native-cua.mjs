/** Native check evidence. UI actions must be supplied by the Computer Use caller. */
import fs from "node:fs/promises";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { isDeepStrictEqual } from "node:util";
import { capture, openFixture } from "./editing-cua.mjs";

async function persist(check) {
  await fs.writeFile(path.join(check.directory, "result.json"), JSON.stringify(check, null, 2));
}

function archivedValue(value) {
  const serialized=JSON.stringify(value,(_, member) => {
    if (["undefined","function","symbol","bigint"].includes(typeof member)
        || (typeof member === "number" && !Number.isFinite(member))) {
      throw new Error("Native assertion requires finite JSON values");
    }
    return member;
  });
  return JSON.parse(serialized);
}

export async function beginNativeCheck(app, manifestPath, identifier) {
  const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
  const specification = manifest.required_native_checks.find(check => check.id === identifier);
  if (!specification || !/^[a-z0-9-]+$/.test(identifier)) throw new Error("Unknown native check");
  const root = path.dirname(manifestPath);
  const directory = path.join(root, "native-checks", identifier);
  try {
    await fs.access(directory);
    const attempts = path.join(root, "native-attempts");
    await fs.mkdir(attempts, { recursive: true });
    await fs.rename(directory, path.join(attempts, `${identifier}-${randomUUID()}`));
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  await fs.mkdir(directory, { recursive: true });
  const replayPath = path.join(directory, `${identifier}-${randomUUID()}.md`);
  const check = { run_id: manifest.run_id, id: identifier, build: manifest.bundle,
    specification, directory, replayPath, replayValid: false, passed: false,
    replayStage: "preparing", actions: [], observations: [], assertions: [] };
  await fs.writeFile(replayPath, specification.source);
  await persist(check);
  try {
    await openFixture(app, replayPath);
    check.replayStage = "observing";
    await persist(check);
    return check;
  } catch (error) {
    check.error = String(error);
    await persist(check);
    throw error;
  }
}

export async function observeNativeStage(app, check, stage, description) {
  if (check.replayStage !== "observing") throw new Error("Native check is not observing");
  const expectedStage = check.specification.stages[check.observations.length];
  if (stage !== expectedStage) throw new Error(`Expected native stage ${expectedStage}, received ${stage}`);
  await capture(app, check.directory, stage);
  check.observations.push({ stage, description });
  await persist(check);
}

export async function assertNativeValue(check, name, actual, expected) {
  if (check.replayStage !== "observing" || !name || actual === undefined || expected === undefined) {
    throw new Error("Native assertion requires an active check and explicit values");
  }
  check.assertions.push({ name, actual: archivedValue(actual), expected: archivedValue(expected) });
  await persist(check);
}

export async function recordNativeAction(check, description) {
  if (check.replayStage !== "observing" || !description) throw new Error("Native action requires an active check");
  check.actions.push(description);
  await persist(check);
}

export async function finishNativeCheck(check) {
  if (check.replayStage !== "observing") throw new Error("Native check is not observing");
  if (!isDeepStrictEqual(check.observations.map(observation => observation.stage), check.specification.stages)) {
    throw new Error("Native observations are incomplete");
  }
  if (check.assertions.length === 0) throw new Error("Native assertions are missing");
  for (const stage of check.specification.stages) {
    for (const suffix of [".png", ".ax.txt"]) await fs.access(path.join(check.directory, stage + suffix));
  }
  check.replayValid = true;
  check.passed = check.assertions.every(assertion => isDeepStrictEqual(assertion.actual, assertion.expected));
  check.replayStage = "completed";
  await persist(check);
  return check;
}
