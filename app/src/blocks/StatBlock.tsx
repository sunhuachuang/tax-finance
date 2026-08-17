/**
 * 数字块。值只能来自 binding —— 属性面板里没有「值」这一栏，
 * 用户能改的是标题、单位和强调色，不是数字本身。
 */
import { z } from "zod";

import { registerBlock, type BlockViewProps } from "../core/registry";
import { formatValue } from "./format";

const propsSchema = z.object({
  suffix: z.string().default(""),
  tone: z.enum(["neutral", "attention"]).default("neutral"),
});

function StatBlock({ block, value, text, loading, error }: BlockViewProps) {
  const props = propsSchema.parse(block.props);
  const binding = block.binding;

  return (
    <div className={`block-body stat tone-${props.tone}`}>
      <div className="block-title">{text("title")}</div>
      <div className="stat-value">
        {error ? "—" : loading && value === undefined ? "…" : formatValue(value)}
        {props.suffix ? <span className="stat-suffix">{props.suffix}</span> : null}
      </div>
      {/* 数据块必须标注来源：数字是从哪条查询来的，看得见。 */}
      <div className="block-provenance">
        {binding ? `${binding.source}.${binding.path || "*"}${binding.agg === "count" ? " · 计数" : ""}` : "未绑定数据源"}
      </div>
    </div>
  );
}

registerBlock({
  type: "stat",
  name: "数字",
  hint: "单个指标。值来自绑定的查询，不可手填。",
  kind: "data",
  propsSchema,
  defaultProps: { suffix: "", tone: "neutral" },
  defaultBinding: { source: "overview", path: "review_drafts", agg: "count" },
  defaultSpan: { desktop: 3, mobile: 2 },
  copy: [{ key: "title", label: "标题", fallback: "待审草稿" }],
  fields: [
    { key: "suffix", label: "后缀", control: "text", placeholder: "如「笔」" },
    {
      key: "tone",
      label: "强调",
      control: "select",
      options: [
        { value: "neutral", label: "普通" },
        { value: "attention", label: "需要注意" },
      ],
    },
  ],
  Component: StatBlock,
});
