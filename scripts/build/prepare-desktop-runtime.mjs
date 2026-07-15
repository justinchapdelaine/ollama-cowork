import { copyFileSync, mkdirSync } from "node:fs"
import { spawnSync } from "node:child_process"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const release = process.argv.includes("--release")
const cargo = process.platform === "win32" ? "cargo.exe" : "cargo"
const rustc = process.platform === "win32" ? "rustc.exe" : "rustc"
const metadataResult = spawnSync(
  cargo,
  ["metadata", "--format-version", "1", "--no-deps"],
  { cwd: repository, encoding: "utf8" },
)
if (metadataResult.status !== 0) {
  process.stderr.write(metadataResult.stderr ?? "cargo metadata failed\n")
  process.exit(metadataResult.status ?? 1)
}
const targetDirectory = JSON.parse(metadataResult.stdout).target_directory
const hostResult = spawnSync(rustc, ["-vV"], { cwd: repository, encoding: "utf8" })
if (hostResult.status !== 0) {
  process.stderr.write(hostResult.stderr ?? "rustc host discovery failed\n")
  process.exit(hostResult.status ?? 1)
}
const hostTriple = /^host:\s*(\S+)$/m.exec(hostResult.stdout)?.[1]
if (!hostTriple) throw new Error("rustc did not report a host target triple")

const tauriTarget = process.env.TAURI_ENV_TARGET_TRIPLE
const configuredTarget = process.env.CARGO_BUILD_TARGET
for (const target of [tauriTarget, configuredTarget]) {
  if (target && !/^[A-Za-z0-9_.-]+$/.test(target)) {
    throw new Error(`unsupported Cargo target triple: ${target}`)
  }
}
// Tauri reports the host triple to every hook, including ordinary native runs.
// Only cross-compile when the requested target actually differs from rustc's
// host (or Cargo explicitly configured a target); otherwise reuse target/debug
// or target/release and stage the small helper files into both layouts.
const buildTarget = configuredTarget ?? (tauriTarget && tauriTarget !== hostTriple ? tauriTarget : undefined)
const args = [
  "build",
  "-p",
  "ollama-cowork-broker-host",
  "-p",
  "ollama-cowork-docx-tool",
]
if (release) args.push("--release")
if (buildTarget) args.push("--target", buildTarget)

const result = spawnSync(cargo, args, { cwd: repository, stdio: "inherit" })
if (result.status !== 0) process.exit(result.status ?? 1)

const profile = release ? "release" : "debug"
const builtOutput = buildTarget
  ? join(targetDirectory, buildTarget, profile)
  : join(targetDirectory, profile)
const destinations = new Set([join(targetDirectory, profile), builtOutput])

// Tauri exposes the resolved target triple to hooks even for a default host
// build. Stage the same trusted assets in the non-triple layout as well so the
// resolver works for both default and explicit `tauri --target` invocations.
if (tauriTarget) destinations.add(join(targetDirectory, tauriTarget, profile))

const executableSuffix = process.platform === "win32" ? ".exe" : ""
const runtimeFiles = [
  `ollama-cowork-broker-host${executableSuffix}`,
  `ollama-cowork-docx-tool${executableSuffix}`,
]
for (const destination of destinations) {
  mkdirSync(destination, { recursive: true })
  for (const runtimeFile of runtimeFiles) {
    const source = join(builtOutput, runtimeFile)
    const target = join(destination, runtimeFile)
    if (source !== target) copyFileSync(source, target)
  }
  copyFileSync(
    join(repository, "scripts", "runtime", "srt-docx-bridge.mjs"),
    join(destination, "srt-docx-bridge.mjs"),
  )
}
