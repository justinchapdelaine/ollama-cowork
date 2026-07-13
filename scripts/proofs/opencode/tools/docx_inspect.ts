import { tool } from "@opencode-ai/plugin"

const endpoint = () => {
  const url = process.env.SPIKE_BROKER_URL
  const token = process.env.SPIKE_BROKER_AUTH
  if (!url || !token || !/^http:\/\/127\.0\.0\.1:\d+$/.test(url)) throw new Error("trusted broker environment is invalid")
  return { url, token }
}

export default tool({
  description: "Inspect the selected DOCX through the trusted local broker.",
  args: {},
  async execute() {
    const { url, token } = endpoint()
    const response = await fetch(`${url}/execute`, { method: "POST", headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" }, body: JSON.stringify({ schema_version: 1, operation: "inspect" }) })
    if (!response.ok) throw new Error(`trusted broker rejected inspect: ${response.status}`)
    return JSON.stringify(await response.json())
  },
})
