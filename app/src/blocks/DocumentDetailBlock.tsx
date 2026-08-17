/**
 * 文档详情：这份文件是什么，谁读过它，读出了什么，以及哪里没对上。
 *
 * **这里没有「触发提取」按钮，是有意的。** 读文档要模型，模型在 agent 侧
 * （crabtalk / Claude 经 MCP 的 `record_reading` 提交读数）。app 里放一个
 * 「提取」按钮就意味着 app 自己要接一个模型，那是另一个决定，不该从一个
 * 按钮里溜进来。app 能做的是：把状态说清楚，以及承接真正属于人的那两个决定。
 */
import { useState } from "react";
import { z } from "zod";

import * as ipc from "../core/ipc";
import { registerBlock, type BlockViewProps } from "../core/registry";
import { formatValue } from "./format";
import { STATUS_LABELS } from "./DocumentListBlock";

const propsSchema = z.object({
  showRawPayload: z.boolean().default(false),
});

type Issue = { severity: "error" | "warning"; code: string; message: string };

type Extraction = {
  id: string;
  version: number;
  extracted_by: string;
  extracted_at: string;
  reported_confidence: number | null;
  issues: Issue[];
  superseded: boolean;
  payload: Record<string, unknown>;
};

type Detail = {
  document: {
    id: string;
    original_filename: string | null;
    mime: string;
    byte_len: number;
    sha256: string;
    received_at: string;
    status: string;
    source: { kind: string };
    stored_path: string;
  };
  extractions: Extraction[];
  local_path: string | null;
};

function DocumentDetailBlock({ block, value, text, loading, error, editing, refresh }: BlockViewProps) {
  const props = propsSchema.parse(block.props);
  const detail = value as Detail | null;

  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  if (!detail) {
    return (
      <div className="block-body doc-detail">
        <div className="block-title">{text("title")}</div>
        {error ? <div className="block-error">{error}</div> : null}
        <div className="block-empty">{loading ? "读取中…" : text("empty")}</div>
      </div>
    );
  }

  const doc = detail.document;

  async function act(fn: () => Promise<unknown>) {
    setBusy(true);
    setActionError(null);
    try {
      await fn();
      refresh();
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(false);
    }
  }

  // 只有 store 白名单允许的迁移才给按钮。
  const canIgnore = doc.status !== "ignored";
  const canRequeue = doc.status === "ignored";

  return (
    <div className="block-body doc-detail">
      <div className="block-title">{text("title")}</div>
      {error ? <div className="block-error">{error}</div> : null}
      {actionError ? <div className="block-error">{actionError}</div> : null}

      <div className="doc-headline">
        <span className="doc-headline-name">{doc.original_filename || "（无文件名）"}</span>
        <span className={`doc-status status-${doc.status}`}>
          {STATUS_LABELS[doc.status] ?? doc.status}
        </span>
      </div>

      <dl className="doc-meta">
        <dt>类型</dt>
        <dd>{doc.mime}</dd>
        <dt>大小</dt>
        <dd>{doc.byte_len.toLocaleString()} bytes</dd>
        <dt>来源</dt>
        <dd>{doc.source?.kind ?? "—"}</dd>
        <dt>收到</dt>
        <dd>{doc.received_at}</dd>
        {/* 内容哈希就是这份文档的身份，去重也是按它。 */}
        <dt>sha256</dt>
        <dd className="doc-hash">{doc.sha256}</dd>
      </dl>

      <div className="doc-actions">
        <button
          type="button"
          className="btn"
          disabled={editing || busy || !detail.local_path}
          title={detail.local_path ?? "远程模式下文件在 host 上，这台机器打不开"}
          onClick={() => void act(() => ipc.openDocument(doc.id))}
        >
          用系统程序打开
        </button>
        {canIgnore ? (
          <button
            type="button"
            className="btn"
            disabled={editing || busy}
            title="重复件、私人收据、垃圾邮件——排除掉，但不删除"
            onClick={() => void act(() => ipc.setDocumentStatus(doc.id, "ignored"))}
          >
            忽略
          </button>
        ) : null}
        {canRequeue ? (
          <button
            type="button"
            className="btn"
            disabled={editing || busy}
            onClick={() => void act(() => ipc.setDocumentStatus(doc.id, "pending_extraction"))}
          >
            放回待提取
          </button>
        ) : null}
      </div>

      <div className="doc-section-label">
        读数（extraction）
        <span className="doc-count">{detail.extractions.length}</span>
      </div>

      {detail.extractions.length === 0 ? (
        <div className="block-empty">
          还没有任何 agent 读过这份文档。读文档由 MCP 的 <code>record_reading</code> 提交，
          不在这个界面里发生。
        </div>
      ) : (
        <ul className="extraction-list">
          {detail.extractions.map((ex) => (
            <li key={ex.id} className={`extraction ${ex.superseded ? "superseded" : ""}`}>
              <div className="extraction-head">
                <span className="extraction-version">v{ex.version}</span>
                <span className="extraction-by">{ex.extracted_by}</span>
                {ex.reported_confidence !== null ? (
                  // 置信度是模型的自述，只作参考——决定权在 issues。
                  <span className="extraction-confidence">
                    自述置信度 {(ex.reported_confidence * 100).toFixed(0)}%
                  </span>
                ) : null}
                {ex.superseded ? <span className="extraction-superseded">已被更新版本取代</span> : null}
                <span className="extraction-at">{ex.extracted_at?.slice(0, 19).replace("T", " ")}</span>
              </div>

              <div className="extraction-payload">
                <span>{String(ex.payload.supplier_name ?? "（无供应商）")}</span>
                <span>{String(ex.payload.invoice_number ?? "")}</span>
                <span>{String(ex.payload.invoice_date ?? "")}</span>
                <span className="extraction-total">{formatValue(ex.payload.total)}</span>
                {ex.payload.gst != null ? (
                  <span>GST {formatValue(ex.payload.gst)}</span>
                ) : null}
              </div>

              {ex.issues.length > 0 ? (
                <ul className="issue-list">
                  {ex.issues.map((issue) => (
                    <li key={issue.code} className={`issue ${issue.severity}`}>
                      <span className="issue-severity">
                        {issue.severity === "error" ? "错误" : "警告"}
                      </span>
                      <code className="issue-code">{issue.code}</code>
                      <span className="issue-message">{issue.message}</span>
                    </li>
                  ))}
                </ul>
              ) : (
                <div className="issue-clean">算术校验通过，没有问题</div>
              )}

              {props.showRawPayload ? (
                <pre className="extraction-raw">{JSON.stringify(ex.payload, null, 2)}</pre>
              ) : null}
            </li>
          ))}
        </ul>
      )}

      <div className="block-provenance">
        提取由 agent 经 MCP 的 record_reading 提交；这里只呈现结果和人的决定（忽略 / 放回队列）。
      </div>
    </div>
  );
}

registerBlock({
  type: "document-detail",
  name: "文档详情",
  hint: "选中文档的元信息、读数与校验问题，以及忽略 / 放回队列。",
  // action 而不是 gate：忽略和放回队列会写，但不产生任何账。
  // gate 留给唯一那个把草稿变成账的闸口。
  kind: "action",
  propsSchema,
  defaultProps: { showRawPayload: false },
  defaultBinding: { source: "document", path: "", agg: "value", params: { id: "$document" } },
  defaultSpan: { desktop: 7, mobile: 4 },
  copy: [
    { key: "title", label: "标题", fallback: "文档详情" },
    { key: "empty", label: "未选中时的文案", fallback: "在左边选一份文档。" },
  ],
  fields: [{ key: "showRawPayload", label: "显示原始 payload", control: "toggle" }],
  Component: DocumentDetailBlock,
});
