import { readFile, writeFile } from "node:fs/promises";

const request = JSON.parse(await readFile(process.argv[2], "utf8"));
if (request.schema_version !== 1) throw new Error("unsupported permission proof schema");
if (!/^http:\/\/127\.0\.0\.1:\d+$/.test(request.base_url)) throw new Error("base_url must be loopback IPv4");
if (!new Set(["once", "reject", "abort"]).has(request.action)) throw new Error("invalid action");

const headers = { Authorization: request.authorization };
const jsonHeaders = { ...headers, "Content-Type": "application/json" };
const timeoutMs = request.timeout_ms ?? 120_000;
const controller = new AbortController();
const events = [];
const seenEventTypes = [];
let permission = null;
let responseEvent = null;
let abortResult = null;
let streamError = null;
let brokerDecision = null;

async function checkedFetch(path, init = {}) {
  const response = await fetch(request.base_url + path, { ...init, headers: { ...headers, ...(init.headers ?? {}) } });
  if (!response.ok) throw new Error(`${init.method ?? "GET"} ${path} returned ${response.status}: ${await response.text()}`);
  return response;
}

async function decideBroker(actionId, decision) {
  if (!request.broker_control_url || !request.broker_control_authorization || !request.broker_job_id) return;
  const response = await fetch(request.broker_control_url + "/decision", {
    method: "POST",
    headers: {
      Authorization: request.broker_control_authorization,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      schema_version: 1,
      job_id: request.broker_job_id,
      action_id: actionId,
      decision,
    }),
  });
  if (!response.ok) throw new Error(`broker decision returned ${response.status}: ${await response.text()}`);
  brokerDecision = await response.json();
}

async function handleEvent(event) {
  event = event?.payload ?? event;
  if (event?.type && seenEventTypes.length < 200) seenEventTypes.push(event.type);
  if (event?.properties?.sessionID !== request.session_id && event?.properties?.sessionID !== undefined) return false;
  if ((event?.type === "permission.asked" || event?.type === "permission.updated") && event.properties.sessionID === request.session_id && !permission) {
    permission = event.properties;
    events.push({ type: event.type, permission_id: permission.id, permission_type: permission.type, title: permission.title });
    if (request.action === "abort") {
      await decideBroker(permission.id, "cancelled");
      const response = await checkedFetch(`/session/${request.session_id}/abort`, { method: "POST" });
      abortResult = await response.json();
    } else {
      await decideBroker(permission.id, request.action === "once" ? "approved_once" : "rejected");
      await checkedFetch(`/permission/${permission.id}/reply`, {
        method: "POST",
        headers: jsonHeaders,
        body: JSON.stringify({ reply: request.action }),
      });
    }
    return false;
  }
  if (event?.type === "permission.replied" && event.properties.sessionID === request.session_id) {
    responseEvent = event.properties;
    events.push({ type: event.type, permission_id: responseEvent.permissionID ?? responseEvent.requestID, response: responseEvent.response ?? responseEvent.reply });
    return false;
  }
  if (event?.type === "session.idle" && event.properties.sessionID === request.session_id) {
    events.push({ type: event.type });
    return true;
  }
  return false;
}

const streamPromise = (async () => {
  try {
    const response = await checkedFetch("/event", { signal: controller.signal });
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const frames = buffer.split(/\r?\n\r?\n/);
      buffer = frames.pop() ?? "";
      for (const frame of frames) {
        const data = frame.split(/\r?\n/).filter((line) => line.startsWith("data:")).map((line) => line.slice(5).trim()).join("\n");
        if (!data) continue;
        if (await handleEvent(JSON.parse(data))) return;
      }
    }
  } catch (error) {
    if (error?.name !== "AbortError") streamError = `${error.name}: ${error.message}`;
  }
})();

await new Promise((resolve) => setTimeout(resolve, 100));
const prompt = request.prompt ?? `You must call the available ${request.tool_name} tool exactly once now. Pass ${request.argument_name}=${request.token}. Do not answer COMPLETE unless that tool returns successfully; do not substitute a text answer for the tool call. After a successful tool result answer exactly COMPLETE.`;
await checkedFetch(`/session/${request.session_id}/prompt_async`, {
  method: "POST",
  headers: jsonHeaders,
  body: JSON.stringify({
    model: { providerID: "ollama-lan", modelID: "gemma4:12b" },
    parts: [{ type: "text", text: prompt }],
  }),
});

let timedOut = false;
try {
  await Promise.race([streamPromise, new Promise((_, reject) => setTimeout(() => reject(new Error(`permission proof timed out after ${timeoutMs}ms`)), timeoutMs))]);
} catch (error) {
  timedOut = true;
  streamError = `${error.name}: ${error.message}`;
}
controller.abort();
await streamPromise;

const messages = await (await checkedFetch(`/session/${request.session_id}/message`)).json();
const parts = messages.flatMap((message) => message.parts ?? []);
const toolParts = parts.filter((part) => part.type === "tool" && part.tool === request.tool_name);
const completedToolParts = toolParts.filter((part) => part.state?.status === "completed");
const expectedOutput = request.output_contains ?? `${request.execution_marker}${request.token}`;
const executed = completedToolParts.some((part) => String(part.state?.output).includes(expectedOutput));
const finalText = parts.filter((part) => part.type === "text").map((part) => part.text).join("\n");
const passed = request.action === "once"
  ? Boolean(permission && (responseEvent?.response ?? responseEvent?.reply) === "once" && executed && finalText.includes("COMPLETE"))
  : request.action === "reject"
    ? Boolean(permission && (responseEvent?.response ?? responseEvent?.reply) === "reject" && !executed)
    : Boolean(permission && abortResult === true && !executed);

const result = {
  schema_version: 1,
  action: request.action,
  session_id: request.session_id,
  permission_requested: Boolean(permission),
  permission_id: permission?.id ?? null,
  permission_type: permission?.type ?? null,
  response_event: responseEvent,
  abort_result: abortResult,
  tool_part_count: toolParts.length,
  completed_tool_count: completedToolParts.length,
  executed,
  final_text: finalText,
  stream_error: streamError,
  timed_out: timedOut,
  seen_event_types: seenEventTypes,
  events,
  broker_decision: brokerDecision,
  passed,
};
await writeFile(request.result_path, JSON.stringify(result, null, 2));
process.stdout.write(`${JSON.stringify(result)}\n`);
process.exitCode = passed ? 0 : 1;
