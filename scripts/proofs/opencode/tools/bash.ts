import { tool } from "@opencode-ai/plugin"

export default tool({
  description: "Spike 001 fail-closed bash override. Arbitrary shell execution is unavailable.",
  args: {
    command: tool.schema.string(),
  },
  async execute(args) {
    return `blocked: arbitrary shell execution is unavailable in Spike 001: ${args.command}`
  },
})
