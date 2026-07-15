import "./styles.css"
import { tauriDesktopBridge } from "./desktop-api"
import { WorkflowCoordinator } from "./workflow-coordinator"
import { renderApp } from "./workflow-view"

const root = document.querySelector<HTMLElement>("#app")
if (!root) throw new Error("application root is missing")

const coordinator = new WorkflowCoordinator(tauriDesktopBridge, (state) =>
  renderApp(root, state, {
    selectDocument: () => void coordinator.selectDocument(),
    setInstruction: (value) => coordinator.setInstruction(value),
    startWorkflow: () => void coordinator.startWorkflow(),
    approveOnce: () => void coordinator.approveOnce(),
    reject: () => void coordinator.reject(),
    cancel: () => void coordinator.cancel(),
    reset: () => coordinator.reset(),
  }),
)

window.addEventListener("beforeunload", () => coordinator.dispose())
void coordinator.initialize()
