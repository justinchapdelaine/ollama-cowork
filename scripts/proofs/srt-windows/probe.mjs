import { appendFile, readFile, writeFile } from "node:fs/promises";
import http from "node:http";
import net from "node:net";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

function fail(message, error) {
  const detail = error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  process.stderr.write(`${message}: ${detail}\n`);
  process.exitCode = 1;
}

async function directHttp(url) {
  await new Promise((resolve, reject) => {
    const request = http.get(url, { timeout: 3000 }, (response) => {
      response.resume();
      response.on("end", resolve);
    });
    request.on("timeout", () => request.destroy(new Error("timeout")));
    request.on("error", reject);
  });
}

async function directTcp(host, port) {
  delete process.env.HTTP_PROXY;
  delete process.env.HTTPS_PROXY;
  delete process.env.ALL_PROXY;
  delete process.env.http_proxy;
  delete process.env.https_proxy;
  delete process.env.all_proxy;

  await new Promise((resolve, reject) => {
    const socket = net.createConnection({ host, port: Number(port), timeout: 3000 });
    socket.once("connect", () => {
      socket.destroy();
      resolve();
    });
    socket.once("timeout", () => socket.destroy(new Error("timeout")));
    socket.once("error", reject);
  });
}

async function spawnChild(target) {
  await new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [fileURLToPath(import.meta.url), "write", target], {
      stdio: "inherit",
      windowsHide: true,
    });
    child.once("error", reject);
    child.once("exit", (code) => (code === 0 ? resolve() : reject(new Error(`child exit ${code}`))));
  });
}

async function main() {
  const [operation, ...args] = process.argv.slice(2);
  switch (operation) {
    case "read":
      await readFile(args[0]);
      break;
    case "write":
      await writeFile(args[0], "SRT_PROBE_WRITE\n", { flag: "wx" });
      break;
    case "modify":
      await appendFile(args[0], "SRT_PROBE_FORBIDDEN_MUTATION\n");
      break;
    case "http":
      await directHttp(args[0]);
      break;
    case "tcp":
      await directTcp(args[0], args[1]);
      break;
    case "child-write":
      await spawnChild(args[0]);
      break;
    case "sleep-write":
      await new Promise((resolve) => setTimeout(resolve, Number(args[1])));
      await writeFile(args[0], "SRT_TIMEOUT_PROCESS_SURVIVED\n", { flag: "wx" });
      break;
    default:
      throw new Error(`unknown operation: ${operation}`);
  }
  process.stdout.write(`operation succeeded: ${operation}\n`);
}

main().catch((error) => fail("operation failed", error));
