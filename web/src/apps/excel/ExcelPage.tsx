import { OfficePage } from "../shared/OfficePage";

/** 表格(Excel / .xlsx)页面。 */
export function ExcelPage() {
  return (
    <OfficePage
      title="表格 · Excel"
      subtitle="上传 .xlsx 文件,识别并(占位)渲染。后续将接入公式计算内核。"
      accept=".xlsx"
    />
  );
}
