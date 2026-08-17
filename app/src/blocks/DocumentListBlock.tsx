/**
 * 文档列表。点一行就把它写进页面参数，详情块跟着变。
 *
 * 两个块之间没有任何直接联系：这个块写 `setParam("document", id)`，
 * 详情块的 binding 里写 `$document`。中间人是布局文档，不是组件树。
 */
import { z } from "zod";

import { registerBlock, type BlockViewProps } from "../core/registry";

const propsSchema = z.object({
  /** 选中后写进哪个页面参数。默认 `document`。 */
  paramKey: z.string().default("document"),
  status: z
    .enum(["all", "pending_extraction", "extracted", "needs_review", "ignored"])
    .default("all"),
  maxRows: z.number().int().min(1).max(200).default(15),
});

type Doc = {
  id: string;
  original_filename: string | null;
  mime: string;
  byte_len: number;
  received_at: string;
  status: string;
};

export const STATUS_LABELS: Record<string, string> = {
  pending_extraction: "待提取",
  extracted: "已提取",
  needs_review: "需复核",
  ignored: "已忽略",
};

function DocumentListBlock({ block, value, text, loading, error, params, setParam }: BlockViewProps) {
  const props = propsSchema.parse(block.props);
  const all = (Array.isArray(value) ? value : []) as Doc[];
  const docs = props.status === "all" ? all : all.filter((d) => d.status === props.status);
  const selected = params[props.paramKey];

  return (
    <div className="block-body doc-list">
      <div className="block-title">{text("title")}</div>

      {error ? <div className="block-error">{error}</div> : null}

      {loading && all.length === 0 ? (
        <div className="block-empty">读取中…</div>
      ) : docs.length === 0 ? (
        <div className="block-empty">{text("empty")}</div>
      ) : (
        <div className="table-scroll">
          <table className="doc-table">
            <thead>
              <tr>
                <th>文件</th>
                <th>状态</th>
                <th>大小</th>
                <th>收到</th>
              </tr>
            </thead>
            <tbody>
              {docs.slice(0, props.maxRows).map((doc) => (
                <tr
                  key={doc.id}
                  className={`doc-row ${selected === doc.id ? "selected" : ""}`}
                  onClick={() => setParam(props.paramKey, doc.id)}
                >
                  <td className="doc-name">{doc.original_filename || "（无文件名）"}</td>
                  <td>
                    <span className={`doc-status status-${doc.status}`}>
                      {STATUS_LABELS[doc.status] ?? doc.status}
                    </span>
                  </td>
                  <td className="doc-size">{formatBytes(doc.byte_len)}</td>
                  <td className="doc-date">{doc.received_at?.slice(0, 10)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <div className="block-provenance">
        点一行选中它 · 写入页面参数 <code>{props.paramKey}</code>
        {docs.length > props.maxRows ? ` · 共 ${docs.length} 份，显示前 ${props.maxRows} 份` : ""}
      </div>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

registerBlock({
  type: "document-list",
  name: "文档列表",
  hint: "收进来的文档。点一行选中，详情块跟着变。",
  kind: "data",
  propsSchema,
  defaultProps: { paramKey: "document", status: "all", maxRows: 15 },
  defaultBinding: { source: "overview", path: "documents", agg: "value" },
  defaultSpan: { desktop: 5, mobile: 4 },
  copy: [
    { key: "title", label: "标题", fallback: "文档" },
    { key: "empty", label: "空态文案", fallback: "还没有文档。" },
  ],
  fields: [
    { key: "paramKey", label: "写入的页面参数", control: "text", placeholder: "document" },
    {
      key: "status",
      label: "只看",
      control: "select",
      options: [
        { value: "all", label: "全部" },
        { value: "pending_extraction", label: "待提取" },
        { value: "extracted", label: "已提取" },
        { value: "needs_review", label: "需复核" },
        { value: "ignored", label: "已忽略" },
      ],
    },
    { key: "maxRows", label: "最多行数", control: "number", min: 1, max: 200 },
  ],
  Component: DocumentListBlock,
});
