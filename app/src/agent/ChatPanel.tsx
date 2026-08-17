/**
 * 和 AI 助手对话。这是改界面和问数据的主入口。
 *
 * 模型调用全在 Rust 侧——这个组件只发一句话、渲染记录。API key 不经过 webview，
 * 页面里任何东西（包括 agent 自己生成的布局）都拿不到。
 *
 * 对话记录和事件订阅在 `store.ts`，不在这里：关掉面板不该丢记录，
 * 也不该丢正在流的那半句回复。
 */
import { useEffect, useRef, useState } from "react";

import { useSettings } from "../settings/store";
import { useChat } from "./store";

/** 工具名 → 给人看的说法。 */
const TOOL_LABELS: Record<string, string> = {
  get_overview: "读账本总览",
  get_gst_return: "算 GST 申报表",
  get_ir3_summary: "算 IR3 汇总",
  get_document: "读文档",
  get_layout: "读当前布局",
  list_block_types: "查可用块型",
  apply_layout: "改布局",
};

export function ChatPanel() {
  const turns = useChat((s) => s.turns);
  const busy = useChat((s) => s.busy);
  const ready = useChat((s) => s.ready);
  const send = useChat((s) => s.send);
  const reset = useChat((s) => s.reset);

  const showSettings = useSettings((s) => s.show);
  const [input, setInput] = useState("");
  const [showThinking, setShowThinking] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);

  useEffect(() => {
    scroller.current?.scrollTo({ top: scroller.current.scrollHeight });
  }, [turns]);

  // ready === null 表示还没问过 Rust，这时不要抢先说「不可用」。
  const blocked = ready === false;
  const canSend = ready === true && !busy && input.trim().length > 0;

  function submit() {
    if (!canSend) return;
    const text = input.trim();
    setInput("");
    void send(text);
  }

  return (
    <aside className="chat">
      <div className="panel-title">
        AI 助手
        <label className="chat-toggle">
          <input
            type="checkbox"
            checked={showThinking}
            onChange={(e) => setShowThinking(e.target.checked)}
          />
          显示思考
        </label>
        <button
          type="button"
          className="btn subtle"
          disabled={turns.length === 0}
          onClick={() => void reset()}
        >
          新对话
        </button>
      </div>

      {/* 没配 key 时界面照常显示，只是发不出去——横幅说明原因。 */}
      {blocked ? (
        <div className="chat-notice">
          <strong>还没有设置 Claude API Key</strong>
          <p>填一把 key 就能用，立刻生效，不用重启。</p>
          <button type="button" className="btn" onClick={() => void showSettings()}>
            去设置
          </button>
        </div>
      ) : null}

      <div className="chat-scroll" ref={scroller}>
        {turns.length === 0 ? (
          <div className="chat-empty">
            <p>统计一下我的财务和税务情况，或者让我改变一下页面布局。比如：</p>
            <ul>
              <li>这一期 GST 要交多少？这个数是怎么来的？</li>
              <li>这个税年的收入、支出和净利润各是多少？</li>
              <li>待审的草稿里有哪些看着像重复的？</li>
              <li>把 GST 那三个数字挪到总览页最上面</li>
              <li>新建一页叫「银行」，放未对账流水的表</li>
            </ul>
          </div>
        ) : (
          turns.map((turn, i) => (
            <div key={i} className={`chat-turn ${turn.role}`}>
              {turn.thinking && showThinking ? (
                <div className="chat-thinking">{turn.thinking}</div>
              ) : null}

              {turn.tools.map((call, j) => (
                <div key={j} className={`chat-tool ${call.ok ? "" : "failed"}`}>
                  <span className="chat-tool-name">{TOOL_LABELS[call.name] ?? call.name}</span>
                  {call.summary ? <span className="chat-tool-summary">{call.summary}</span> : null}
                  {!call.ok ? <span className="chat-tool-failed">未成功</span> : null}
                </div>
              ))}

              {/* 退避重试期间界面会静默好几秒，不说一声会像卡死。 */}
              {turn.notices.map((notice, j) => (
                <div key={`n${j}`} className="chat-notice-line">
                  {notice}
                </div>
              ))}

              {turn.text ? <div className="chat-text">{turn.text}</div> : null}

              {/* 思考期间既没有正文也没有工具——不给个信号会像卡住了。 */}
              {turn.role === "agent" && !turn.text && turn.tools.length === 0 && turn.notices.length === 0 ? (
                <div className="chat-pending">{turn.thinking ? "思考中…" : "…"}</div>
              ) : null}
            </div>
          ))
        )}
      </div>

      <div className="chat-input">
        <textarea
          value={input}
          placeholder={
            blocked
              ? "设置 API key 后可用"
              : busy
                ? "正在处理…"
                : "问问财务和税务，或让我改页面布局"
          }
          rows={3}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            // Enter 发送，Shift+Enter 换行。
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              submit();
            }
          }}
        />
        <button
          type="button"
          className="btn active"
          disabled={!canSend}
          title={blocked ? "还没有设置 API key" : undefined}
          onClick={submit}
        >
          {busy ? "…" : "发送"}
        </button>
      </div>
    </aside>
  );
}
