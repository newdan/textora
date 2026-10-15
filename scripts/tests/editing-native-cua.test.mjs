import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { runInNewContext } from "node:vm";
import { beginNativeCheck, observeNativeStage, assertNativeValue, finishNativeCheck } from "../editing-native-cua.mjs";

async function fixture(context) {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "textora-native-check-"));
  context.after(() => fs.rm(directory, { recursive: true, force: true }));
  const manifestPath = path.join(directory, "manifest.json");
  await fs.writeFile(manifestPath, JSON.stringify({run_id:"test-run", bundle:{executable_sha256:"test-build"},
    required_native_checks:[{id:"system-ime",source:"\ufefforiginal\r\n",stages:["preedit","commit"]}]}));
  const app = {
    surface:"editor", fixturePath:"",
    async getAXState() {
      if (this.surface === "goto") return "1 text field ID: PathTextField";
      if (this.surface === "panel") return "2 button Open, ID: OKButton";
      return `0 window ${path.basename(this.fixturePath)}`;
    },
    async getAXStateAndScreenshot() {return {state:await this.getAXState(),screenshot:Buffer.from("test-only pixels")};},
    async pressKey(key) {
      if (key === "super+o" || key === "Return") this.surface="panel";
      if (key === "super+shift+g") this.surface="goto";
    },
    async setValue(_, value) {this.fixturePath=value;},
    async click() {this.surface="editor";}
  };
  return {app,manifestPath};
}

test("native assertions compare archived values across JavaScript realms", async context => {
  const {app,manifestPath}=await fixture(context);
  const check=await beginNativeCheck(app,manifestPath,"system-ime");
  for (const stage of check.specification.stages) await observeNativeStage(app,check,stage,"observed");
  const external=runInNewContext('({ files: [], geometry: { width: 2022, heights: [1648] } })');
  await assertNativeValue(check,"files",external.files,[]);
  await assertNativeValue(check,"geometry",external.geometry,{width:2022,heights:[1648]});
  external.files.push("changed after observation");
  await finishNativeCheck(check);
  assert.equal(check.passed,true);
  const archived=JSON.parse(await fs.readFile(path.join(check.directory,"result.json"),"utf8"));
  assert.deepEqual(archived.assertions[0].actual,[]);
});

test("native assertions reject values that cannot be archived faithfully", async context => {
  const {app,manifestPath}=await fixture(context);
  const check=await beginNativeCheck(app,manifestPath,"system-ime");
  await assert.rejects(assertNativeValue(check,"invalid number",Number.NaN,null),/JSON values/);
  await assert.rejects(assertNativeValue(check,"missing nested value",{width:undefined},{}),/JSON values/);
  assert.deepEqual(check.assertions,[]);
});

test("native retries preserve prior evidence and use unique byte-exact fixtures", async context => {
  const {app,manifestPath}=await fixture(context);
  const first=await beginNativeCheck(app,manifestPath,"system-ime");
  const second=await beginNativeCheck(app,manifestPath,"system-ime");
  assert.notEqual(first.replayPath,second.replayPath);
  assert.equal(await fs.readFile(second.replayPath,"utf8"),"\ufefforiginal\r\n");
  const attempts=await fs.readdir(path.join(path.dirname(manifestPath),"native-attempts"));
  assert.equal(attempts.length,1);
  const old=JSON.parse(await fs.readFile(path.join(path.dirname(manifestPath),"native-attempts",attempts[0],"result.json")));
  assert.equal(old.replayValid,false);
  assert.equal(old.build.executable_sha256,"test-build");
});

test("native observations reject skipped or duplicate stages without sending input", async context => {
  const {app,manifestPath}=await fixture(context);
  const check=await beginNativeCheck(app,manifestPath,"system-ime");
  await assert.rejects(observeNativeStage(app,check,"commit","skipped preedit"),/Expected native stage preedit/);
  await observeNativeStage(app,check,"preedit","actual composition");
  await assert.rejects(observeNativeStage(app,check,"preedit","duplicate"),/Expected native stage commit/);
  assert.equal(check.observations.length,1);
});

test("native completion requires observations, assertions and original capture files", async context => {
  const {app,manifestPath}=await fixture(context);
  const check=await beginNativeCheck(app,manifestPath,"system-ime");
  await assert.rejects(finishNativeCheck(check),/observations are incomplete/);
  for (const stage of check.specification.stages) await observeNativeStage(app,check,stage,"observed");
  await assert.rejects(finishNativeCheck(check),/assertions are missing/);
  await assertNativeValue(check,"saved source","actual","actual");
  await fs.unlink(path.join(check.directory,"preedit.png"));
  await assert.rejects(finishNativeCheck(check),/ENOENT/);
  assert.equal(check.replayValid,false);
});

test("native assertions cannot pass unequal values or be changed after completion", async context => {
  const {app,manifestPath}=await fixture(context);
  const check=await beginNativeCheck(app,manifestPath,"system-ime");
  for (const stage of check.specification.stages) await observeNativeStage(app,check,stage,"observed");
  await assert.rejects(assertNativeValue(check,"empty",undefined,true),/explicit values/);
  await assertNativeValue(check,"saved source","duplicated","once");
  await finishNativeCheck(check);
  assert.equal(check.replayValid,true);
  assert.equal(check.passed,false);
  await assert.rejects(assertNativeValue(check,"overwrite",true,true),/active check/);
  await assert.rejects(finishNativeCheck(check),/not observing/);
});
