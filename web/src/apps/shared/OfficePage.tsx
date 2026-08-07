import { FileUpload } from "./FileUpload";
import { useOfficeFile } from "./useOfficeFile";

interface OfficePageProps {
  /** 页面标题,如「文档 (Word)」。 */
  title: string;
  /** 页面说明。 */
  subtitle: string;
  /** 接受的扩展名,如 ".docx"。 */
  accept: string;
}

/**
 * 单个 office 组件页面的通用框架:
 * 标题 + 上传入口 + 解析结果展示区。
 * 三个页面(Word/Excel/PPT)都基于它,各自具备独立上传入口。
 */
export function OfficePage({ title, subtitle, accept }: OfficePageProps) {
  const { state, openFile } = useOfficeFile();

  return (
    <section className="office-page" aria-label={title}>
      <header className="office-page__header">
        <h2>{title}</h2>
        <p className="office-page__subtitle">{subtitle}</p>
      </header>

      <div className="office-page__upload">
        <FileUpload accept={accept} onFile={openFile} label={`上传 ${accept} 文件`} />
        {state.fileName && <span className="office-page__filename">{state.fileName}</span>}
      </div>

      <div className="office-page__result" data-testid="result">
        {state.loading && <p>正在解析…</p>}
        {state.error && <p className="office-page__error">解析失败:{state.error}</p>}
        {state.result && (
          <>
            <dl className="office-page__meta">
              <dt>识别格式</dt>
              <dd>{state.result.format_name}</dd>
              <dt>文件大小</dt>
              <dd>{state.result.byte_len} 字节</dd>
              <dt>解析状态</dt>
              <dd>{state.result.ok ? "成功" : "失败"}</dd>
            </dl>
            <p className={state.result.ok ? "office-page__summary" : "office-page__error"}>
              {state.result.message}
            </p>
          </>
        )}
        {!state.loading && !state.error && !state.result && (
          <p className="office-page__empty">尚未选择文件。请上传一个 {accept} 文件以查看识别结果。</p>
        )}
      </div>
    </section>
  );
}
