import { describe, expect, it } from "vitest";

import {
  type ConversationMessage,
  type SessionSnapshot,
  type ToolCall,
  type WorkspaceSelection,
  SessionController,
} from "./session";

describe("SessionController", () => {
  it("activates a loaded session and exposes active workspace state", () => {
    const controller = new SessionController();
    const message = textMessage("user", "hello");

    controller.activateSession(
      sessionSnapshot({
        id: "session-1",
        messages: [message],
      }),
      workspaceSelection("workspace-1", "C:\\work\\project"),
    );

    expect(controller.activeSessionId).toBe("session-1");
    expect(controller.selectedWorkspaceId).toBe("workspace-1");
    expect(controller.selectedWorkspaceRoot).toBe("C:\\work\\project");
    expect(controller.baseUrl).toBe("http://127.0.0.1:11434");
    expect(controller.model).toBe("gemma4:12b");
    expect(controller.messages).toEqual([message]);
  });

  it("creates run contexts from the currently active session", () => {
    const controller = new SessionController();

    expect(controller.createRunContext()).toBeNull();

    controller.activateSession(
      sessionSnapshot({ id: "session-2" }),
      workspaceSelection("workspace-2", "C:\\work\\project"),
    );

    const context = controller.createRunContext();

    expect(context?.sessionId).toBe("session-2");
    expect(context?.workspaceId).toBe("workspace-2");
    expect(context?.runId).toEqual(expect.any(String));
  });

  it("reconciles streamed placeholder messages with final run messages", () => {
    const controller = activeController();
    const streamedUser = textMessage("user", "streamed prompt");
    const streamedAssistant = message("assistant", []);
    const finalUser = textMessage("user", "streamed prompt");
    const finalAssistant = textMessage("assistant", "final answer");
    const activeIds = new Set([streamedUser.id, streamedAssistant.id]);

    controller.appendMessage(streamedUser);
    controller.appendMessage(streamedAssistant);
    controller.reconcileRunMessages(activeIds, [finalUser, finalAssistant]);

    expect(controller.messages).toEqual([finalUser, finalAssistant]);
  });

  it("removes only empty active assistant messages", () => {
    const controller = activeController();
    const activeEmptyAssistant = message("assistant", []);
    const inactiveEmptyAssistant = message("assistant", []);
    const activeAssistantWithText = textMessage("assistant", "still here");

    controller.replaceMessages([
      activeEmptyAssistant,
      inactiveEmptyAssistant,
      activeAssistantWithText,
    ]);

    const changed = controller.removeEmptyAssistantMessages(
      new Set([activeEmptyAssistant.id, activeAssistantWithText.id]),
    );

    expect(changed).toBe(true);
    expect(controller.messages).toEqual([inactiveEmptyAssistant, activeAssistantWithText]);
  });

  it("appends thinking, text, and tool call parts to the target message", () => {
    const controller = activeController();
    const assistant = message("assistant", []);
    const call: ToolCall = {
      id: "call-1",
      name: "list_files",
      arguments: { path: "." },
    };

    controller.replaceMessages([assistant]);
    controller.appendTextPart(assistant.id, "thinking", "plan");
    controller.appendTextPart(assistant.id, "thinking", " more");
    controller.appendTextPart(assistant.id, "text", "answer");
    controller.appendToolCall(assistant.id, call);

    expect(controller.messages[0].parts).toEqual([
      { type: "thinking", text: "plan more" },
      { type: "text", text: "answer" },
      { type: "tool_call", call },
    ]);
  });
});

function activeController(): SessionController {
  const controller = new SessionController();
  controller.activateSession(
    sessionSnapshot({ id: "session-active" }),
    workspaceSelection("workspace-active", "C:\\work\\project"),
  );
  return controller;
}

function sessionSnapshot(
  overrides: Partial<SessionSnapshot> = {},
): SessionSnapshot {
  return {
    id: "session",
    title: "Project",
    workspaceRoot: "C:\\work\\project",
    baseUrl: "http://127.0.0.1:11434",
    model: "gemma4:12b",
    createdAtMs: 1,
    updatedAtMs: 1,
    messages: [],
    ...overrides,
  };
}

function workspaceSelection(id: string, sourceRoot: string): WorkspaceSelection {
  return { id, source_root: sourceRoot };
}

function textMessage(
  role: ConversationMessage["role"],
  text: string,
): ConversationMessage {
  return message(role, [{ type: "text", text }]);
}

function message(
  role: ConversationMessage["role"],
  parts: ConversationMessage["parts"],
): ConversationMessage {
  return {
    id: crypto.randomUUID(),
    role,
    parts,
  };
}
