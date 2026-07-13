import { tool } from "@opencode-ai/plugin"

export default tool({
  description: "Create a revised DOCX copy by replacing one unambiguous Heading1 section through the trusted broker.",
  args: { heading: tool.schema.string(), replacement_paragraphs: tool.schema.array(tool.schema.string()).min(1).max(32) },
  async execute(args, context) {
    await context.ask({ permission: "docx_rewrite_section", patterns: [args.heading], always: [], metadata: { heading: args.heading, paragraph_count: args.replacement_paragraphs.length } })
    const url = process.env.SPIKE_BROKER_URL
    const token = process.env.SPIKE_BROKER_AUTH
    if (!url || !token || !/^http:\/\/127\.0\.0\.1:\d+$/.test(url)) throw new Error("trusted broker environment is invalid")
    const response = await fetch(`${url}/execute`, { method: "POST", headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" }, body: JSON.stringify({ schema_version: 1, operation: "rewrite_section", heading: args.heading, replacement_paragraphs: args.replacement_paragraphs }) })
    if (!response.ok) throw new Error(`trusted broker rejected rewrite: ${response.status}`)
    return JSON.stringify(await response.json())
  },
})
