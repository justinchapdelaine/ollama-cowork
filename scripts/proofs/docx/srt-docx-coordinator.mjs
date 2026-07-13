import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { SandboxManager } from "@anthropic-ai/sandbox-runtime";

const request = JSON.parse(await readFile(process.argv[2], "utf8"));
const sha256 = async (path) => createHash("sha256").update(await readFile(path)).digest("hex");

let requestSequence = 0;
async function invoke(input) {
  const inputPath = `${request.request_directory}\\tool-request-${++requestSequence}.json`;
  await writeFile(inputPath, JSON.stringify(input));
  const command = `"${request.executable.replaceAll('"', '\\"')}" --request "${inputPath.replaceAll('"', '\\"')}"`;
  const { argv, env } = await SandboxManager.wrapWithSandboxArgv(command, undefined, undefined, undefined, request.cwd);
  return new Promise((resolve, reject) => {
    const child = spawn(argv[0], argv.slice(1), { cwd: request.cwd, env, windowsHide: true, stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "", stderr = "";
    child.stdout.on("data", (v) => stdout += v); child.stderr.on("data", (v) => stderr += v);
    child.once("error", reject); child.once("exit", (code) => resolve({ code, stdout, stderr }));
  });
}

let resetError = null;
const sourceBefore = await sha256(request.source);
let rewrite, validate, overwrite;
try {
  await SandboxManager.initialize(request.settings);
  rewrite = await invoke({ schema_version: 1, operation: "rewrite_section", input: request.source, output: request.output, heading: "Executive Summary", replacement_paragraphs: request.replacement });
  validate = await invoke({ schema_version: 1, operation: "validate", input: request.output });
  overwrite = await invoke({ schema_version: 1, operation: "rewrite_section", input: request.source, output: request.output, heading: "Executive Summary", replacement_paragraphs: ["must not overwrite"] });
} finally {
  try { await SandboxManager.reset(); } catch (error) { resetError = String(error); }
}
const sourceAfter = await sha256(request.source);
const rewriteJson = JSON.parse(rewrite?.stdout || "null");
const validateJson = JSON.parse(validate?.stdout || "null");
const overwriteJson = JSON.parse(overwrite?.stdout || "null");
const adjacent = validateJson?.sections?.find((s) => s.heading === "Operating Constraints")?.paragraphs ?? [];
const passed = rewrite?.code === 0 && validate?.code === 0 && overwrite?.code !== 0 && overwriteJson?.status === "error" && sourceBefore === sourceAfter && adjacent.includes("SPIKE001-ORIGINAL-CANARY-7F93D1") && resetError === null;
const report = { generated_at: new Date().toISOString(), source_sha256_before: sourceBefore, source_sha256_after: sourceAfter, rewrite: rewriteJson, validation: validateJson, overwrite_denial: overwriteJson, reset_error: resetError, passed };
await writeFile(request.report, JSON.stringify(report, null, 2));
process.stdout.write(`${JSON.stringify(report, null, 2)}\n`); process.exitCode = passed ? 0 : 1;
