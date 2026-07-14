import { copyFileSync, mkdirSync } from "node:fs"
import { spawnSync } from "node:child_process"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "../..")
const release = process.argv.includes("--release")
const cargo = process.platform === "win32" ? "cargo.exe" : "cargo"
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
const targetTriple = process.env.TAURI_ENV_TARGET_TRIPLE ?? process.env.CARGO_BUILD_TARGET
if (targetTriple && !/^[A-Za-z0-9_.-]+$/.test(targetTriple)) {
  throw new Error(`unsupported Cargo target triple: ${targetTriple}`)
}
const args = [
  "build",
  "-p",
  "ollama-cowork-broker-host",
  "-p",
  "ollama-cowork-docx-tool",
]
if (release) args.push("--release")
if (targetTriple) args.push("--target", targetTriple)

const result = spawnSync(cargo, args, { cwd: repository, stdio: "inherit" })
if (result.status !== 0) process.exit(result.status ?? 1)

const profile = release ? "release" : "debug"
const builtOutput = targetTriple
  ? join(targetDirectory, targetTriple, profile)
  : join(targetDirectory, profile)
const destinations = new Set([builtOutput])

// Tauri exposes the resolved target triple to hooks even for a default host
// build. Stage the same trusted assets in the non-triple layout as well so the
// resolver works for both default and explicit `tauri --target` invocations.
if (process.env.TAURI_ENV_TARGET_TRIPLE) {
  destinations.add(join(targetDirectory, profile))
}

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
