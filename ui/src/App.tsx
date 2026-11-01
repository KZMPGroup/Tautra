import { useEffect, useState } from "react";
import { Sidebar, type Page } from "./components/Sidebar";
import { notifyUiReady } from "./lib/agent";
import type { UiInfo } from "./lib/types";
import { Devices } from "./pages/Devices";
import { General } from "./pages/General";
import { Home } from "./pages/Home";
import { Input } from "./pages/Input";
import { Layout } from "./pages/Layout";
import { Sharing } from "./pages/Sharing";

export default function App({ info }: { info: UiInfo }) {
  const [page, setPage] = useState<Page>("home");

  useEffect(() => {
    notifyUiReady().catch((err) => console.error("ui_ready gagal", err));
  }, []);

  const background = info.mica ? "bg-transparent" : "bg-neutral-100 dark:bg-neutral-900";

  return (
    <div className={`flex h-screen text-neutral-900 dark:text-neutral-100 ${background}`}>
      <Sidebar page={page} onNavigate={setPage} />
      <main className="min-w-0 flex-1 overflow-y-auto rounded-ss-xl bg-white/40 ring-1 ring-black/5 dark:bg-black/15 dark:ring-white/5">
        <div className="mx-auto w-full max-w-4xl px-8 py-7">
          {page === "home" && <Home onNavigate={setPage} />}
          {page === "devices" && <Devices />}
          {page === "layout" && <Layout />}
          {page === "input" && <Input />}
          {page === "sharing" && <Sharing />}
          {page === "general" && <General info={info} />}
        </div>
      </main>
    </div>
  );
}
