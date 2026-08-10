import { useState } from "react";
import { WordPage } from "@tengxiaohyx/office-word";
import { ExcelPage } from "./apps/excel/ExcelPage";
import { PptPage } from "./apps/ppt/PptPage";
import "@tengxiaohyx/office-shared/page.css";
import "./App.css";

type TabKey = "word" | "excel" | "ppt";

const TABS: { key: TabKey; label: string }[] = [
  { key: "word", label: "文档" },
  { key: "excel", label: "表格" },
  { key: "ppt", label: "演示" },
];

export default function App() {
  const [tab, setTab] = useState<TabKey>("word");
  // 表格页要让 canvas 撑满视口剩余高度,需要一条从 body 到画布的完整高度链
  const isSheetTab = tab === "excel";

  return (
    <div className={isSheetTab ? "app app--fill" : "app"}>
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

      {/* 表格页要尽可能大的绘制区域,单独放宽容器 */}
      <main className={isSheetTab ? "app__main app__main--wide" : "app__main"}>
        {tab === "word" && <WordPage />}
        {tab === "excel" && <ExcelPage />}
        {tab === "ppt" && <PptPage />}
      </main>
    </div>
  );
}
