/**
 * 申报表块：GST101 和 IR3 汇总共用。
 *
 * 两者的形状是一样的——一串 `ReturnLine`，每行带 `contributions`——
 * 差别只在表头的元信息，所以这里按「有什么渲染什么」处理，而不是写两个块。
 *
 * 每一行都可以展开看到背后的分录。这才是引擎真正的产品：
 * 不是那个数字，而是几年后还能回答「这个数字哪来的」。
 */
import { Fragment, useState } from "react";
import { z } from "zod";

import { registerBlock, type BlockViewProps } from "../core/registry";
import { formatValue } from "./format";

const propsSchema = z.object({
  /** 默认展开所有行的出处。行多的时候会很长，所以默认关。 */
  expandAll: z.boolean().default(false),
});

type Contribution = {
  entry: string;
  amount: unknown;
  narration: string;
  sources?: { kind: string; id?: string }[];
};

type ReturnLine = {
  code: string;
  label: string;
  amount: unknown;
  contributions?: Contribution[];
};

type TaxReturn = {
  lines?: ReturnLine[];
  period?: { start: string; end: string };
  year?: string;
  due?: string;
  self_filed_due?: string;
  rules_year?: string;
  rules_version?: number;
  warnings?: string[];
  notes?: string[];
};

function TaxReturnBlock({ block, value, text, loading, error }: BlockViewProps) {
  const props = propsSchema.parse(block.props);
  const ret = (value ?? {}) as TaxReturn;
  const lines = ret.lines ?? [];

  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const isOpen = (code: string) => props.expandAll || expanded.has(code);

  function toggle(code: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(code)) next.delete(code);
      else next.add(code);
      return next;
    });
  }

  // 两种申报表的表头字段不同，有哪个显示哪个。
  const meta = [
    ret.period ? `期间 ${ret.period.start} → ${ret.period.end}` : null,
    ret.year ? `税年 ${ret.year}` : null,
    ret.due ? `截止 ${ret.due}` : null,
    ret.self_filed_due ? `自行申报截止 ${ret.self_filed_due}` : null,
    ret.rules_year ? `规则 ${ret.rules_year} v${ret.rules_version ?? "?"}` : null,
  ].filter(Boolean) as string[];

  const remarks = [...(ret.warnings ?? []), ...(ret.notes ?? [])];

  return (
    <div className="block-body return-block">
      <div className="block-title">{text("title")}</div>

      {error ? <div className="block-error">{error}</div> : null}

      {meta.length > 0 ? (
        <div className="return-meta">
          {meta.map((item) => (
            <span key={item}>{item}</span>
          ))}
        </div>
      ) : null}

      {loading && lines.length === 0 ? (
        <div className="block-empty">读取中…</div>
      ) : lines.length === 0 ? (
        <div className="block-empty">{error ? "取不到数据。" : text("empty")}</div>
      ) : (
        <table className="return-table">
          <tbody>
            {lines.map((line) => {
              const contributions = line.contributions ?? [];
              const open = isOpen(line.code);
              return (
                <Fragment key={line.code}>
                  <tr
                    className={`return-row ${contributions.length > 0 ? "expandable" : ""}`}
                    onClick={contributions.length > 0 ? () => toggle(line.code) : undefined}
                  >
                    <td className="return-code">{line.code.split(".").pop()}</td>
                    <td className="return-label">{line.label}</td>
                    <td className="return-amount">{formatValue(line.amount)}</td>
                    <td className="return-toggle">
                      {contributions.length > 0 ? (
                        <span>
                          {open ? "▾" : "▸"} {contributions.length}
                        </span>
                      ) : (
                        // 零贡献不是错误：这一期没有相应的分录，或者这一行是小计。
                        <span className="return-empty-mark">—</span>
                      )}
                    </td>
                  </tr>

                  {open && contributions.length > 0 ? (
                    <tr className="return-detail-row">
                      <td colSpan={4}>
                        <ul className="provenance-list">
                          {contributions.map((c) => (
                            <li key={c.entry}>
                              <span className="prov-amount">{formatValue(c.amount)}</span>
                              <span className="prov-narration">{c.narration}</span>
                              <span className="prov-sources">
                                {(c.sources ?? []).map((s, i) => (
                                  <span
                                    key={`${s.kind}-${s.id ?? i}`}
                                    // manual = 没有任何文档支撑，必须看得出来。
                                    className={`prov-source ${s.kind === "manual" ? "unbacked" : ""}`}
                                    title={s.id ?? undefined}
                                  >
                                    {s.kind === "manual" ? "无凭证" : s.kind}
                                  </span>
                                ))}
                              </span>
                            </li>
                          ))}
                        </ul>
                      </td>
                    </tr>
                  ) : null}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      )}

      {remarks.length > 0 ? (
        <ul className="return-remarks">
          {remarks.map((r) => (
            <li key={r}>{r}</li>
          ))}
        </ul>
      ) : null}

      <div className="block-provenance">
        {block.binding ? `${block.binding.source}${paramSuffix(block)}` : "未绑定数据源"}
        {lines.length > 0 ? " · 点一行看它的出处" : ""}
      </div>
    </div>
  );
}

function paramSuffix(block: BlockViewProps["block"]): string {
  const entries = Object.entries(block.binding?.params ?? {});
  return entries.length === 0 ? "" : `(${entries.map(([k, v]) => `${k}=${v}`).join(", ")})`;
}

registerBlock({
  type: "tax-return",
  name: "申报表",
  hint: "GST101 / IR3 的各行，点开看每个数字背后的分录。",
  kind: "data",
  propsSchema,
  defaultProps: { expandAll: false },
  defaultBinding: { source: "gst", path: "", agg: "value", params: { date: "$date" } },
  defaultSpan: { desktop: 12, mobile: 4 },
  copy: [
    { key: "title", label: "标题", fallback: "GST101" },
    { key: "empty", label: "空态文案", fallback: "这一期没有数据。" },
  ],
  fields: [{ key: "expandAll", label: "默认展开全部出处", control: "toggle" }],
  Component: TaxReturnBlock,
});
