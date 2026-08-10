import { memo, useEffect, useMemo, useState } from "react";
import type { FilterSpec, SheetHandle, UniqueValues } from "./sheet";
import { columnLabel } from "./grid/labels";

/** 值集过滤枚举的唯一值上限(超出后提示,避免超大列卡住 UI)。 */
const UNIQUE_LIMIT = 500;

interface FilterBarProps {
  sheet: SheetHandle;
  /** 当前选中列(0 基)—— 过滤面板作用于它。 */
  activeCol: number;
  /** 当前各列已生效的过滤(0 基列 → 规格)。 */
  filters: Map<number, FilterSpec>;
  /** 顶部作为表头、始终保留的行数。 */
  headerRows: number;
  /** 设置某列过滤(spec 为 null 表示清除该列)。 */
  onApply: (col: number, spec: FilterSpec | null) => void;
  /** 清除所有列过滤。 */
  onClearAll: () => void;
}

type Kind = FilterSpec["kind"];

const KIND_LABEL: Record<Kind, string> = {
  text: "文本",
  number: "数值",
  values: "值集",
  blank: "空白",
};

const TEXT_OPS: { value: string; label: string }[] = [
  { value: "contains", label: "包含" },
  { value: "notContains", label: "不包含" },
  { value: "equals", label: "等于" },
  { value: "begins", label: "开头是" },
  { value: "ends", label: "结尾是" },
];

const NUM_OPS: { value: string; label: string }[] = [
  { value: "gt", label: ">" },
  { value: "ge", label: "≥" },
  { value: "lt", label: "<" },
  { value: "le", label: "≤" },
  { value: "eq", label: "=" },
  { value: "ne", label: "≠" },
  { value: "between", label: "介于" },
];

/**
 * 列过滤面板。
 *
 * 作用于**当前选中列**:选类型(文本/数值/值集/空白)→ 填条件 → 应用。
 * 生效的过滤以「标签」列出,可单独或整体清除。重扫描在 Rust/WASM 侧完成,
 * 这里只负责收集条件与展示。
 */
function FilterBarInner({
  sheet,
  activeCol,
  filters,
  headerRows,
  onApply,
  onClearAll,
}: FilterBarProps) {
  const [kind, setKind] = useState<Kind>("text");
  const [op, setOp] = useState("contains");
  const [needle, setNeedle] = useState("");
  const [a, setA] = useState("");
  const [b, setB] = useState("");
  const [blankWanted, setBlankWanted] = useState(true);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [unique, setUnique] = useState<UniqueValues>({ values: [], truncated: false });

  const colName = columnLabel(activeCol);

  // 切到值集类型或换列时,拉取该列唯一值
  useEffect(() => {
    if (kind !== "values" || !sheet.uniqueValues) return;
    const u = sheet.uniqueValues(activeCol, headerRows, UNIQUE_LIMIT);
    setUnique(u);
    setChecked(new Set(u.values));
  }, [kind, activeCol, sheet, headerRows]);

  const activeList = useMemo(
    () =>
      [...filters.entries()].sort((x, y) => x[0] - y[0]).map(([col, spec]) => ({ col, spec })),
    [filters],
  );

  const apply = () => {
    let spec: FilterSpec | null = null;
    switch (kind) {
      case "text":
        if (needle.length === 0) return;
        spec = { col: activeCol, kind: "text", op, needle };
        break;
      case "number": {
        const na = Number(a);
        if (a.trim() === "" || !Number.isFinite(na)) return;
        // 空的上界回退到 a(注意 Number("") 是 0 而非 NaN,必须先判空串)
        const nb = b.trim() === "" ? na : Number(b);
        spec = { col: activeCol, kind: "number", op, a: na, b: Number.isFinite(nb) ? nb : na };
        break;
      }
      case "blank":
        spec = { col: activeCol, kind: "blank", blank: blankWanted };
        break;
      case "values":
        spec = { col: activeCol, kind: "values", values: [...checked] };
        break;
    }
    onApply(activeCol, spec);
  };

  return (
    <div className="sheet__filter-bar" data-testid="filter-bar">
      <span className="sheet__filter-title">筛选 {colName} 列</span>

      <select
        aria-label="过滤类型"
        value={kind}
        onChange={(e) => setKind(e.target.value as Kind)}
      >
        {(Object.keys(KIND_LABEL) as Kind[]).map((k) => (
          <option key={k} value={k}>
            {KIND_LABEL[k]}
          </option>
        ))}
      </select>

      {kind === "text" && (
        <>
          <select aria-label="文本运算" value={op} onChange={(e) => setOp(e.target.value)}>
            {TEXT_OPS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
          <input
            aria-label="文本关键字"
            value={needle}
            placeholder="关键字"
            onChange={(e) => setNeedle(e.target.value)}
          />
        </>
      )}

      {kind === "number" && (
        <>
          <select aria-label="数值运算" value={op} onChange={(e) => setOp(e.target.value)}>
            {NUM_OPS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
          <input
            aria-label="数值"
            value={a}
            placeholder="数值"
            inputMode="decimal"
            onChange={(e) => setA(e.target.value)}
          />
          {op === "between" && (
            <input
              aria-label="数值上界"
              value={b}
              placeholder="上界"
              inputMode="decimal"
              onChange={(e) => setB(e.target.value)}
            />
          )}
        </>
      )}

      {kind === "blank" && (
        <select
          aria-label="空白筛选"
          value={blankWanted ? "blank" : "nonblank"}
          onChange={(e) => setBlankWanted(e.target.value === "blank")}
        >
          <option value="blank">只看空白</option>
          <option value="nonblank">只看非空白</option>
        </select>
      )}

      {kind === "values" && (
        <div className="sheet__filter-values" role="group" aria-label="值集">
          {unique.values.length === 0 && <span className="sheet__filter-hint">(该列无可选值)</span>}
          {unique.values.map((v) => (
            <label key={v} className="sheet__filter-check">
              <input
                type="checkbox"
                checked={checked.has(v)}
                onChange={(e) => {
                  setChecked((prev) => {
                    const next = new Set(prev);
                    if (e.target.checked) next.add(v);
                    else next.delete(v);
                    return next;
                  });
                }}
              />
              {v || "(空)"}
            </label>
          ))}
          {unique.truncated && <span className="sheet__filter-hint">值过多,仅列出前 {UNIQUE_LIMIT} 个</span>}
        </div>
      )}

      <button type="button" onClick={apply} data-testid="filter-apply">
        应用
      </button>
      {filters.has(activeCol) && (
        <button type="button" className="sheet__filter-link" onClick={() => onApply(activeCol, null)}>
          清除本列
        </button>
      )}

      {activeList.length > 0 && (
        <span className="sheet__filter-active" data-testid="filter-active">
          已筛选:
          {activeList.map(({ col }) => (
            <button
              key={col}
              type="button"
              className="sheet__filter-chip"
              title="点击清除该列筛选"
              onClick={() => onApply(col, null)}
            >
              {columnLabel(col)} ✕
            </button>
          ))}
          <button type="button" className="sheet__filter-link" onClick={onClearAll}>
            全部清除
          </button>
        </span>
      )}
    </div>
  );
}

/**
 * 过滤面板。用 `memo` 包一层:它挂在 `SheetCanvas` 下,而后者会随渲染器统计更新
 * 重渲染;面板自身最多列 500 个值集复选框,跟着一起重建的代价明显。
 * props 全是稳定引用(`useCallback` / `useMemo`),浅比较足够。
 */
export const FilterBar = memo(FilterBarInner);
