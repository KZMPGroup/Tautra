import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { AgentProvider, getUiInfo } from "./lib/agent";
import { I18nProvider } from "./i18n";
import "./index.css";

async function boot() {
  const info = await getUiInfo().catch((err) => {
    console.error("ui_info gagal", err);
    return { mica: false, version: "" };
  });
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <AgentProvider>
        <I18nProvider>
          <App info={info} />
        </I18nProvider>
      </AgentProvider>
    </StrictMode>,
  );
}

void boot();
