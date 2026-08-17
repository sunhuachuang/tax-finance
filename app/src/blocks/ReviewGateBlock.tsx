/**
 * 人工确认闸口。
 *
 * 这是整个系统里唯一能把草稿变成账的地方——MCP 工具表刻意没有 approve/reject。
 * 因此该块型 `locked`：布局怎么改都不能把它删掉，标题文案也不开放改写，
 * 免得被改成看不出后果的措辞（见 ARCHITECTURE.md 约束 3）。
 */
import { useState } from "react";
import { z } from "zod";

import * as ipc from "../core/ipc";
import { registerBlock, type BlockViewProps } from "../core/registry";
import { formatValue } from "./format";

const propsSchema = z.object({
  maxRows: z.number().int().min(1).max(50).default(8),
});

type Draft = {
  id: string;
  date: string;
  narration: string;
  postings?: { account: string; amount: unknown }[];
};

function ReviewGateBlock({ block, value, loading, error, editing, refresh }: BlockViewProps) {
  const props = propsSchema.parse(block.props);
  const drafts = (Array.isArray(value) ? value : []) as Draft[];

  const [busy, setBusy] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  async function act(id: string, action: "approve" | "reject") {
    setBusy(id);
    setActionError(null);
    try {
      await (action === "approve" ? ipc.approveEntry(id) : ipc.rejectEntry(id));
      refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="block-body gate">
      <div className="block-title gate-title">
        待确认草稿
        <span className="gate-badge">人工闸口</span>
      </div>

      {error ? <div className="block-error">{error}</div> : null}
      {actionError ? <div className="block-error">{actionError}</div> : null}

      {loading && drafts.length === 0 ? (
        <div className="block-empty">读取中…</div>
      ) : drafts.length === 0 ? (
        <div className="block-empty">没有待确认的草稿。</div>
      ) : (
        <ul className="gate-list">
          {drafts.slice(0, props.maxRows).map((draft) => (
            <li key={draft.id} className="gate-row">
              <div className="gate-row-main">
                <div className="gate-narration">{draft.narration || "（无摘要）"}</div>
                <div className="gate-meta">
                  {draft.date}
                  {draft.postings?.length ? ` · ${draft.postings.length} 条分录` : ""}
                  {draft.postings?.[0] ? ` · ${formatValue(draft.postings[0].amount)}` : ""}
                </div>
              </div>
              <div className="gate-actions">
                <button
                  type="button"
                  className="btn approve"
                  /* 编辑模式下停用真实动作：排版时误点不该改账本。 */
                  disabled={editing || busy === draft.id}
                  title={editing ? "编辑模式下不可操作" : "批准并入账"}
                  onClick={() => void act(draft.id, "approve")}
                >
                  批准
                </button>
                <button
                  type="button"
                  className="btn reject"
                  disabled={editing || busy === draft.id}
                  title={editing ? "编辑模式下不可操作" : "拒绝（作废不删）"}
                  onClick={() => void act(draft.id, "reject")}
                >
                  拒绝
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}

      <div className="block-provenance">
        批准写入账本，拒绝作废但不删除。两个动作都只存在于 app 和 taxweb，不在 MCP 工具表里。
      </div>
    </div>
  );
}

registerBlock({
  type: "review-gate",
  name: "确认闸口",
  hint: "待确认草稿的批准 / 拒绝。锁定块，不可删除。",
  kind: "gate",
  locked: true,
  propsSchema,
  defaultProps: { maxRows: 8 },
  defaultBinding: { source: "overview", path: "review_drafts", agg: "value" },
  defaultSpan: { desktop: 12, mobile: 4 },
  fields: [{ key: "maxRows", label: "最多行数", control: "number", min: 1, max: 50 }],
  Component: ReviewGateBlock,
});
