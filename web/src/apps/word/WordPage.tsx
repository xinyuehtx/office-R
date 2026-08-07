import { OfficePage } from "../shared/OfficePage";

/** 文档(Word / .docx)页面。 */
export function WordPage() {
  return (
    <OfficePage
      title="文档 · Word"
      subtitle="上传 .docx 文件,识别并(占位)渲染。后续将解析段落与样式。"
      accept=".docx"
    />
  );
}
