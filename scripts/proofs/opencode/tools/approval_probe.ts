import { tool } from "@opencode-ai/plugin"

export default tool({
  description: "Harmless ask-gated tool used only to verify one-time approval, rejection, and cancellation.",
  args: {
    token: tool.schema.string(),
  },
  async execute(args, context) {
    await context.ask({
      permission: "approval_probe",
      patterns: [args.token],
      always: [],
      metadata: { token: args.token },
    })
    return `APPROVAL_PROBE_EXECUTED:${args.token}`
  },
})
