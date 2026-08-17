/**
 * 记录表。把 binding 指向的数组渲染成表格。
 *
 * 列不写死：不填就从第一行的键推断。这样账本模型加字段时，
 * 用户不用等发版就能把新字段摆到页面上。
 */
import { z } from "zod";

import { registerBlock, type BlockViewProps } from "../core/registry";
import { formatValue } from "./format";

const propsSchema = z.object({
  /** 逗号分隔的字段名；留空则从数据推断。 */
  columns: z.string().default(""),
  maxRows: z.number().int().min(1).max(200).default(10),
});

function RecordTableBlock({ block, value, text, loading, error }: BlockViewProps) {
  const props = propsSchema.parse(block.props);
  const rows = Array.isArray(value) ? (value as Record<string, unknown>[]) : [];

  const columns = props.columns
    .split(",")
    .map((c) => c.trim())
    .filter(Boolean);
  const effectiveColumns =
    columns.length > 0 ? columns : rows[0] ? Object.keys(rows[0]).slice(0, 6) : [];

  const shown = rows.slice(0, props.maxRows);

  return (
    <div className="block-body table-block">
      <div className="block-title">{text("title")}</div>

      {error ? (
        <div className="block-error">{error}</div>
      ) : loading && rows.length === 0 ? (
        <div className="block-empty">读取中…</div>
      ) : shown.length === 0 ? (
        <div className="block-empty">{text("empty")}</div>
      ) : (
        <div className="table-scroll">
          <table>
            <thead>
              <tr>
                {effectiveColumns.map((col) => (
                  <th key={col}>{col}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {shown.map((row, i) => (
                <tr key={String(row.id ?? i)}>
                  {effectiveColumns.map((col) => (
                    <td key={col}>{formatValue(row[col])}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <div className="block-provenance">
        {block.binding ? `${block.binding.source}.${block.binding.path || "*"}` : "未绑定数据源"}
        {rows.length > shown.length ? ` · 共 ${rows.length} 行，显示前 ${shown.length} 行` : ""}
      </div>
    </div>
  );
}

registerBlock({
  type: "record-table",
  name: "记录表",
  hint: "把一个数组渲染成表格。列可留空自动推断。",
  kind: "data",
  propsSchema,
  defaultProps: { columns: "", maxRows: 10 },
  defaultBinding: { source: "overview", path: "posted_entries", agg: "value" },
  defaultSpan: { desktop: 12, mobile: 4 },
  copy: [
    { key: "title", label: "标题", fallback: "已入账分录" },
    { key: "empty", label: "空态文案", fallback: "没有记录。" },
  ],
  fields: [
    { key: "columns", label: "列（逗号分隔）", control: "text", placeholder: "留空自动推断" },
    { key: "maxRows", label: "最多行数", control: "number", min: 1, max: 200 },
  ],
  Component: RecordTableBlock,
});
