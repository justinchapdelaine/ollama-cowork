import type { ApplicationError } from "./workflow-contracts"

export const normalizeApplicationError = (error: unknown): ApplicationError => {
  if (typeof error === "object" && error !== null) {
    const value = error as Record<string, unknown>
    if (typeof value.code === "string" && typeof value.message === "string") {
      return { code: value.code, message: value.message }
    }
  }
  return {
    code: "desktop_command_failed",
    message: "The desktop command failed.",
  }
}
