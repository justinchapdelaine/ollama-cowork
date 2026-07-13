import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { SandboxManager } from "@anthropic-ai/sandbox-runtime";

const quote = (value) => `"${String(value).replaceAll('"', '\\"')}"`;

async function sha256(path) {
  const bytes = await readFile(path);
  return createHash("sha256").update(bytes).digest("hex").toUpperCase();
}

async function runWrapped(command, cwd, timeoutMs = 15_000) {
  const { argv, env } = await SandboxManager.wrapWithSandboxArgv(
    command,
    undefined,
    undefined,
    undefined,
    cwd,
  );

  return await new Promise((resolve, reject) => {
    const child = spawn(argv[0], argv.slice(1), {
      cwd,
      env,
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill();
    }, timeoutMs);
    child.stdout.on("data", (chunk) => (stdout += chunk));
    child.stderr.on("data", (chunk) => (stderr += chunk));
    child.once("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.once("exit", (code, signal) => {
      clearTimeout(timer);
      if (timedOut) {
        const error = new Error(`probe timed out after ${timeoutMs}ms`);
        error.name = "ProbeTimeoutError";
        reject(error);
      } else {
        resolve({ code: code ?? -1, signal, stdout, stderr });
      }
    });
  });
}

async function main() {
  const requestPath = process.argv[2];
  if (!requestPath) throw new Error("usage: coordinator.mjs <request.json>");
  const request = JSON.parse(await readFile(requestPath, "utf8"));
  const sourceHashBefore = await sha256(request.sourceDocx);
  const results = [];
  let initializationError = null;
  let resetError = null;

  try {
    await SandboxManager.initialize(request.settings);
    for (const probe of request.probes) {
      const command = [quote(request.nodePath), quote(request.probeScript), ...probe.args.map(quote)].join(" ");
      try {
        const observed = await runWrapped(command, request.repoRoot, probe.timeoutMs ?? 15_000);
        const combined = `${observed.stdout}\n${observed.stderr}`.trim();
        const probeReached = /operation (succeeded|failed):/.test(combined);
        const observedSuccess = observed.code === 0;
        results.push({
          name: probe.name,
          expected: probe.expectTimeout ? "timeout" : probe.expectSuccess ? "allow" : "deny",
          exit_code: observed.code,
          signal: observed.signal,
          timed_out: false,
          probe_reached: probeReached,
          passed: probeReached && observedSuccess === probe.expectSuccess,
          stdout: observed.stdout.trim(),
          stderr: observed.stderr.trim(),
        });
      } catch (error) {
        const timedOut = error instanceof Error && error.name === "ProbeTimeoutError";
        results.push({
          name: probe.name,
          expected: probe.expectTimeout ? "timeout" : probe.expectSuccess ? "allow" : "deny",
          exit_code: null,
          signal: null,
          timed_out: timedOut,
          probe_reached: false,
          passed: probe.expectTimeout === true && timedOut,
          stdout: "",
          stderr: error instanceof Error ? `${error.name}: ${error.message}` : String(error),
        });
      }
    }
  } catch (error) {
    initializationError = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  } finally {
    try {
      await Promise.race([
        SandboxManager.reset(),
        new Promise((_, reject) => setTimeout(() => reject(new Error("reset timed out after 30000ms")), 30_000)),
      ]);
    } catch (error) {
      resetError = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
    }
  }

  if (request.postResetWaitMs) {
    await new Promise((resolve) => setTimeout(resolve, request.postResetWaitMs));
  }

  const sourceHashAfter = await sha256(request.sourceDocx);
  const canariesIntact =
    sourceHashBefore === sourceHashAfter &&
    !existsSync(request.outsideWriteTarget) &&
    !existsSync(request.childWriteTarget) &&
    !existsSync(request.timeoutWriteTarget);
  const passed =
    initializationError === null &&
    resetError === null &&
    canariesIntact &&
    results.length === request.probes.length &&
    results.every((result) => result.passed);

  const report = {
    generated_at: new Date().toISOString(),
    srt_version: request.srtVersion,
    srt_win_path: request.settings.windows.srtWin.path,
    node: request.nodePath,
    source_docx: request.sourceDocx,
    source_sha256_before: sourceHashBefore,
    source_sha256_after: sourceHashAfter,
    canaries_intact: canariesIntact,
    initialization_error: initializationError,
    reset_error: resetError,
    settings: request.settings,
    results,
    passed,
  };
  await writeFile(request.reportPath, JSON.stringify(report, null, 2));
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  process.exitCode = passed ? 0 : 1;
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
  process.exitCode = 1;
});
