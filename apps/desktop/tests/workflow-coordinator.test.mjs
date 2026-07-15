import assert from "node:assert/strict"
import test from "node:test"
import { WorkflowCoordinator } from "../src/workflow-coordinator.ts"

const health = {
  appVersion: "0.1.0",
  ready: true,
  opencode: { state: "ready", detail: "ready" },
  sandbox: { state: "ready", detail: "ready" },
  runtimeTools: { state: "ready", detail: "ready" },
  modelEndpoint: { state: "configured", detail: "configured" },
}

class ManualScheduler {
  callbacks = []

  schedule(callback) {
    const handle = { callback }
    this.callbacks.push(handle)
    return handle
  }

  cancel(handle) {
    this.callbacks = this.callbacks.filter((value) => value !== handle)
  }

  async runNext() {
    const handle = this.callbacks.shift()
    assert.ok(handle)
    handle.callback()
    await new Promise((resolve) => setTimeout(resolve, 0))
  }
}

class FakeBridge {
  workflowHandler = () => {}
  polls = []
  approvals = []

  async getHealth() { return health }
  async onHealth() { return () => {} }
  async onWorkflowEvent(handler) {
    this.workflowHandler = handler
    return () => {}
  }
  async selectDocument() {
    return { selectionId: "selection-1", displayName: "input.docx" }
  }
  async startWorkflow() { return { jobId: "job-1" } }
  async approveOnce(jobId, actionId) {
    this.approvals.push([jobId, actionId])
    return { jobId }
  }
  async reject(jobId) { return { jobId } }
  async cancel(jobId) { return { jobId } }
  async poll(jobId) { this.polls.push(jobId) }
}

test("coordinator routes narrow commands and drives polling through injected ports", async () => {
  const bridge = new FakeBridge()
  const scheduler = new ManualScheduler()
  let state
  const coordinator = new WorkflowCoordinator(bridge, (value) => { state = value }, scheduler)

  await coordinator.initialize()
  await coordinator.selectDocument()
  coordinator.setInstruction("Rewrite Summary")
  await coordinator.startWorkflow()
  assert.equal(state.phase, "starting")
  assert.equal(scheduler.callbacks.length, 1)

  await scheduler.runNext()
  assert.deepEqual(bridge.polls, ["job-1"])

  bridge.workflowHandler({
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
  await coordinator.approveOnce()
  assert.deepEqual(bridge.approvals, [["job-1", "action-1"]])

  bridge.workflowHandler({ event: "status_changed", jobId: "job-1", status: "completed" })
  assert.equal(state.phase, "completed")
  assert.equal(scheduler.callbacks.length, 0)
  coordinator.dispose()
})

test("a terminal workflow event is not overwritten by an in-flight poll failure", async () => {
  let workflowHandler
  let rejectPoll
  const states = []
  const bridge = {
    getHealth: async () => health,
    onHealth: async () => () => {},
    onWorkflowEvent: async (handler) => {
      workflowHandler = handler
      return () => {}
    },
    selectDocument: async () => ({ selectionId: "selection-1", displayName: "input.docx" }),
    startWorkflow: async () => ({ jobId: "job-1" }),
    approveOnce: async () => ({ jobId: "job-1" }),
    reject: async () => ({ jobId: "job-1" }),
    cancel: async () => ({ jobId: "job-1" }),
    poll: () => new Promise((_, reject) => {
      rejectPoll = reject
    }),
  }
  const scheduled = []
  const scheduler = {
    schedule: (callback) => {
      scheduled.push(callback)
      return callback
    },
    cancel: () => {},
  }
  const coordinator = new WorkflowCoordinator(bridge, (state) => states.push(state), scheduler)
  await coordinator.initialize()
  await coordinator.selectDocument()
  coordinator.setInstruction("rewrite")
  await coordinator.startWorkflow()
  scheduled.shift()()
  await Promise.resolve()

  workflowHandler({
    event: "failed",
    jobId: "job-1",
    code: "model_event_failed",
    message: "The model event stream failed.",
  })
  rejectPoll({
    code: "workflow_integration_failed",
    message: "The document workflow integration failed.",
  })
  await Promise.resolve()
  await Promise.resolve()

  const latest = states.at(-1)
  assert.equal(latest.phase, "failed")
  assert.deepEqual(latest.error, {
    code: "model_event_failed",
    message: "The model event stream failed.",
  })
  coordinator.dispose()
})

test("dispose during an in-flight poll prevents rescheduling and rendering", async () => {
  let resolvePoll
  const scheduler = new ManualScheduler()
  const bridge = new FakeBridge()
  bridge.poll = () => new Promise((resolve) => { resolvePoll = resolve })
  let renderCount = 0
  const coordinator = new WorkflowCoordinator(
    bridge,
    () => { renderCount += 1 },
    scheduler,
  )
  await coordinator.initialize()
  await coordinator.selectDocument()
  coordinator.setInstruction("rewrite")
  await coordinator.startWorkflow()
  await scheduler.runNext()
  const countAtDispose = renderCount
  coordinator.dispose()
  resolvePoll()
  await Promise.resolve()
  await Promise.resolve()
  assert.equal(scheduler.callbacks.length, 0)
  assert.equal(renderCount, countAtDispose)
})

test("a partial listener setup failure releases the established listener", async () => {
  let healthUnlistenCalls = 0
  const bridge = new FakeBridge()
  bridge.onHealth = async () => () => { healthUnlistenCalls += 1 }
  bridge.onWorkflowEvent = async () => { throw new Error("listener setup failed") }
  const coordinator = new WorkflowCoordinator(bridge, () => {})
  await coordinator.initialize()
  assert.equal(healthUnlistenCalls, 1)
  coordinator.dispose()
  assert.equal(healthUnlistenCalls, 1)
})
