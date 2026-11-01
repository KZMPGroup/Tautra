import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { AgentProvider, getUiInfo } from "./lib/agent";
import { I18nProvider } from "./i18n";
import { loadStrings, matchLanguage, rememberedLanguage } from "./i18n/languages";
import "./index.css";

async function boot() {
  const info = await getUiInfo().catch((err) => {
    console.error("ui_info gagal", err);
    return { mica: false, version: "", languages: [] as string[] };
  });
  const system = matchLanguage([...info.languages, ...navigator.languages]);
  const code = rememberedLanguage() ?? system;
  const initial = { code, t: await loadStrings(code) };
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <AgentProvider>
        <I18nProvider system={system} initial={initial}>
          <App info={info} />
        </I18nProvider>
      </AgentProvider>
    </StrictMode>,
  );
}

void boot();
