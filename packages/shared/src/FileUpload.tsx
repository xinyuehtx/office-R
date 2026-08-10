import { useRef, type ChangeEvent } from "react";

interface FileUploadProps {
  /** 接受的文件扩展名,如 ".docx"。 */
  accept: string;
  /** 选中文件后的回调。 */
  onFile: (file: File) => void;
  /** 按钮文案。 */
  label?: string;
}

/** 通用上传入口:一个按钮触发系统文件选择框。 */
export function FileUpload({ accept, onFile, label = "选择文件" }: FileUploadProps) {
  const inputRef = useRef<HTMLInputElement>(null);

  const handleChange = (e: ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) onFile(file);
    // 允许重复选择同一文件
    e.target.value = "";
  };

  return (
    <label className="file-upload">
      <input
        ref={inputRef}
        type="file"
        accept={accept}
        onChange={handleChange}
        style={{ display: "none" }}
        data-testid="file-input"
      />
      <span className="file-upload__btn">{label}</span>
    </label>
  );
}
