import { OfficePage } from "../shared/OfficePage";

/** 演示(PowerPoint / .pptx)页面。 */
export function PptPage() {
  return (
    <OfficePage
      title="演示 · PowerPoint"
      subtitle="上传 .pptx 文件,识别并(占位)渲染。后续将渲染幻灯片布局。"
      accept=".pptx"
    />
  );
}
