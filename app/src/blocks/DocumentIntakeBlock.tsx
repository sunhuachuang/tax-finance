/**
 * 收文档：选文件，或者直接拖进窗口。
 *
 * 这是个**动作块**——它会写。但写进去的只有 `PendingExtraction` 状态的文档，
 * 没有任何账。提取、分类、生成草稿都在后面的管道里，最后还要过人工闸口。
 *
 * 内容寻址去重在引擎里做：同样的字节进来第二次不会写第二份，
 * 这里如实把「重复」显示出来，而不是假装成功。
 */
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { z } from "zod";

import * as ipc from "../core/ipc";
import { registerBlock, type BlockViewProps } from "../core/registry";

const propsSchema = z.object({
  /** 拖拽收文档。关掉的话只能走文件选择器。 */
  acceptDrop: z.boolean().default(true),
});

const EXTENSIONS = ["pdf", "jpg", "jpeg", "png", "heic", "webp", "csv", "txt"];

type Result = { name: string; status: "stored" | "duplicate" | "failed"; detail?: string };

function DocumentIntakeBlock({ block, text, editing, refresh }: BlockViewProps) {
  const props = propsSchema.parse(block.props);

  const [busy, setBusy] = useState(false);
  const [dropping, setDropping] = useState(false);
  const [results, setResults] = useState<Result[]>([]);

  async function ingest(paths: string[]) {
    if (paths.length === 0) return;
    setBusy(true);
    const collected: Result[] = [];
    for (const path of paths) {
      const name = path.split("/").pop() ?? path;
      try {
        const res = await ipc.ingestDocument(path);
        collected.push({ name, status: res.duplicate ? "duplicate" : "stored" });
      } catch (e) {
        collected.push({ name, status: "failed", detail: String(e) });
      }
    }
    setResults(collected);
    setBusy(false);
    // 总览的计数要跟着动，否则刚收进来的文档看不见。
    refresh();
  }

  async function pick() {
    const picked = await open({
      multiple: true,
      filters: [{ name: "发票 / 收据 / 结单", extensions: EXTENSIONS }],
    });
    if (!picked) return;
    await ingest(Array.isArray(picked) ? picked : [picked]);
  }

  // 原生拖放。Tauri 接管了窗口的拖放，所以走 webview 事件而不是 HTML5 的。
  useEffect(() => {
    if (!props.acceptDrop || editing) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "over") setDropping(true);
        else if (event.payload.type === "leave") setDropping(false);
        else if (event.payload.type === "drop") {
          setDropping(false);
          void ingest(event.payload.paths);
        }
      })
      .then((fn) => {
        // 注册是异步的，期间组件可能已经卸载了。
        if (cancelled) fn();
        else unlisten = fn;
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.acceptDrop, editing]);

  return (
    <div className={`block-body intake ${dropping ? "dropping" : ""}`}>
      <div className="block-title">{text("title")}</div>

      <div className="intake-zone">
        <p className="intake-hint">
          {props.acceptDrop && !editing ? text("hint") : text("hintNoDrop")}
        </p>
        <button type="button" className="btn" disabled={busy || editing} onClick={() => void pick()}>
          {busy ? "收取中…" : "选择文件…"}
        </button>
      </div>

      {results.length > 0 ? (
        <ul className="intake-results">
          {results.map((r) => (
            <li key={`${r.name}-${r.status}`} className={`intake-result ${r.status}`}>
              <span className="intake-name">{r.name}</span>
              <span className="intake-status">
                {r.status === "stored" ? "已收取" : r.status === "duplicate" ? "重复，已跳过" : "失败"}
              </span>
              {r.detail ? <span className="intake-detail">{r.detail}</span> : null}
            </li>
          ))}
        </ul>
      ) : null}

      <div className="block-provenance">
        收进来的文档状态是「待提取」，不产生任何账。提取和生成草稿之后仍要过人工确认闸口。
      </div>
    </div>
  );
}

registerBlock({
  type: "document-intake",
  name: "收文档",
  hint: "上传发票 / 收据 / 结单。只造待提取记录，不产生账。",
  kind: "action",
  propsSchema,
  defaultProps: { acceptDrop: true },
  defaultSpan: { desktop: 12, mobile: 4 },
  copy: [
    { key: "title", label: "标题", fallback: "收文档" },
    { key: "hint", label: "提示文案", fallback: "把发票、收据或银行结单拖进窗口，或者点下面选文件。" },
    { key: "hintNoDrop", label: "禁用拖拽时的文案", fallback: "点下面选择要收取的文件。" },
  ],
  fields: [{ key: "acceptDrop", label: "接受拖拽", control: "toggle" }],
  Component: DocumentIntakeBlock,
});
