import { tool } from "@opencode-ai/plugin"

export default tool({
  description: "Return the supplied value unchanged for a Spike 001 tool-loop proof.",
  args: {
    value: tool.schema.string(),
  },
  async execute(args) {
    return args.value
  },
})
