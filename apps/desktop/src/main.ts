import "./styles.css"
import { getDesktopHealth, onDesktopHealth } from "./desktop-api"
import type { ComponentHealth, DesktopHealth } from "./contracts"

const root = document.querySelector<HTMLElement>("#app")
if (!root) throw new Error("application root is missing")

const componentCard = (name: string, value: ComponentHealth): string => `
  <article class="component component--${value.state}">
    <div class="component__header"><h3>${name}</h3><span>${value.state}</span></div>
    <p>${value.detail}</p>
    ${value.version ? `<code>${value.version}</code>` : ""}
  </article>`

const render = (health: DesktopHealth): void => {
  const modelConfigured = health.modelEndpoint.state === "configured"
  const statusClass = health.ready ? "ready" : modelConfigured ? "pending" : "blocked"
  const statusText = health.ready
    ? "Backend prerequisites ready"
    : modelConfigured
      ? "Local prerequisites ready · model connectivity pending"
      : "Setup required before document workflows"

  root.innerHTML = `
    <section class="shell">
      <header class="hero">
        <p class="eyebrow">OLLAMA COWORK</p>
        <h1>Document workspace</h1>
        <p class="lede">The desktop shell is connected to the trusted Rust composition root.</p>
        <div class="status status--${statusClass}">
          <span class="status__dot"></span>
          ${statusText}
        </div>
      </header>
      <section class="grid" aria-label="Backend health">
        ${componentCard("opencode", health.opencode)}
        ${componentCard("Sandbox Runtime", health.sandbox)}
        ${componentCard("Ollama endpoint", health.modelEndpoint)}
      </section>
      <footer>Desktop ${health.appVersion} · Spike 001 workflow controls are the next milestone.</footer>
    </section>`
}

const renderFailure = (error: unknown): void => {
  root.innerHTML = `<section class="fatal"><h1>Desktop startup failed</h1><p>${String(error)}</p></section>`
}

void onDesktopHealth(render)
void getDesktopHealth().then(render).catch(renderFailure)
