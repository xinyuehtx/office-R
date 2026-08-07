import { useState } from "react";
import { WordPage } from "./apps/word/WordPage";
import { ExcelPage } from "./apps/excel/ExcelPage";
import { PptPage } from "./apps/ppt/PptPage";
import "./App.css";

type TabKey = "word" | "excel" | "ppt";

const TABS: { key: TabKey; label: string }[] = [
  { key: "word", label: "文档" },
  { key: "excel", label: "表格" },
  { key: "ppt", label: "演示" },
];

export default function App() {
  const [tab, setTab] = useState<TabKey>("word");

  return (
    <div className="app">
      <header className="app__bar">
        <h1 className="app__title">office-R</h1>
        <nav className="app__tabs" role="tablist">
          {TABS.map((t) => (
            <button
              key={t.key}
              role="tab"
              aria-selected={tab === t.key}
              className={tab === t.key ? "app__tab app__tab--active" : "app__tab"}
              onClick={() => setTab(t.key)}
            >
              {t.label}
            </button>
          ))}
        </nav>
      </header>

      <main className="app__main">
        {tab === "word" && <WordPage />}
        {tab === "excel" && <ExcelPage />}
        {tab === "ppt" && <PptPage />}
      </main>
    </div>
  );
}
