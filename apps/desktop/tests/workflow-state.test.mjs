import assert from "node:assert/strict"
import test from "node:test"
import {
  acceptStart,
  applyWorkflowEvent,
  initialWorkflowState,
  requestStart,
  resetWorkflow,
  withHealth,
  withInstruction,
  withSelection,
} from "../src/workflow-state.ts"

const health = {
  appVersion: "0.1.0",
  ready: true,
  opencode: { state: "ready", detail: "ready" },
  sandbox: { state: "ready", detail: "ready" },
  runtimeTools: { state: "ready", detail: "ready" },
  modelEndpoint: { state: "configured", detail: "configured" },
}

const prepared = () =>
  withInstruction(
    withSelection(withHealth(initialWorkflowState(), health), {
      selectionId: "selection-1",
      displayName: "input.docx",
    }),
    "Rewrite the Summary",
  )

test("an early workflow event binds the reserved job before invoke resolves", () => {
  const starting = requestStart(prepared())
  const eventFirst = applyWorkflowEvent(starting, {
    event: "status_changed",
    jobId: "job-1",
    status: "running",
  })
  const accepted = acceptStart(eventFirst, "job-1")

  assert.equal(accepted.jobId, "job-1")
  assert.equal(accepted.phase, "running")
  assert.equal(accepted.commandPending, false)
})

test("approval and artifact events form one terminal revised-copy workflow", () => {
  let state = acceptStart(requestStart(prepared()), "job-1")
  state = applyWorkflowEvent(state, {
    event: "action_requested",
    jobId: "job-1",
    action: {
      id: "action-1",
      title: "Create a revised DOCX copy?",
      summary: "Rewrite Summary",
      destructive: false,
      proposal: {
        kind: "docx_section_rewrite",
        heading: "Summary",
        currentParagraphs: ["Current"],
        replacementParagraphs: ["Revised"],
      },
    },
  })
  assert.equal(state.phase, "awaiting_approval")
  assert.equal(state.pendingAction?.id, "action-1")

  state = applyWorkflowEvent(state, {
    event: "artifact_ready",
    jobId: "job-1",
    artifact: { path: "C:/output.revised.docx", mediaType: "application/docx", sha256: "abc" },
  })
  state = applyWorkflowEvent(state, {
    event: "status_changed",
    jobId: "job-1",
    status: "completed",
  })

  assert.equal(state.phase, "completed")
  assert.equal(state.artifact?.sha256, "abc")
  assert.equal(state.pendingAction, null)
})

test("events for another job are ignored and reset clears document authority", () => {
  const active = acceptStart(requestStart(prepared()), "job-1")
  const unchanged = applyWorkflowEvent(active, {
    event: "assistant_text",
    jobId: "job-other",
    partId: "part-other",
    text: "ignore me",
  })
  assert.equal(unchanged, active)

  const reset = resetWorkflow(active)
  assert.equal(reset.phase, "ready")
  assert.equal(reset.selection, null)
  assert.equal(reset.jobId, null)
})

test("assistant deltas remain bounded and preserve distinct parts", () => {
  let state = acceptStart(requestStart(prepared()), "job-1")
  state = applyWorkflowEvent(state, {
    event: "assistant_text",
    jobId: "job-1",
    partId: "part-1",
    text: "A DOCX is ",
  })
  state = applyWorkflowEvent(state, {
    event: "assistant_text",
    jobId: "job-1",
    partId: "part-1",
    text: "already attached.",
  })
  assert.deepEqual(state.assistantText, [
    { partId: "part-1", text: "A DOCX is already attached." },
  ])

  state = applyWorkflowEvent(state, {
    event: "assistant_text",
    jobId: "job-1",
    partId: "part-2",
    text: "A separate assistant part.",
  })
  assert.equal(state.assistantText.length, 2)

  for (let index = 0; index < 300; index += 1) {
    state = applyWorkflowEvent(state, {
      event: "assistant_text",
      jobId: "job-1",
      partId: "part-2",
      text: `${index}:${"x".repeat(2048)}`,
    })
  }
  assert.equal(state.assistantText.length, 2)
  assert.ok(state.assistantText.reduce((total, part) => total + part.text.length, 0) <= 32_768)
})
