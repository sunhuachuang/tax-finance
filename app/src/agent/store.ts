/**
 * 对话记录与事件订阅。
 *
 * 记录**不放在面板组件里**。放在组件里有两个后果：关掉面板再打开，记录就空了，
 * 而 Rust 侧的对话历史还在——模型记得你看不见的内容；回复流到一半关掉面板，
 * 剩下的部分直接丢。所以状态提到这里，事件也在这里订阅一次，
 * 面板只负责渲染。
 *
 * 记录只在内存里，重启应用就没了。Rust 侧的对话历史同理。
 * 要不要把财务对话落盘是另一个决定，暂时不做。
 */
import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";

import * as ipc from "../core/ipc";
import { useEditor } from "../core/store";

export type ToolCall = { name: string; ok: boolean; summary?: string | null };

export type Turn = {
  role: "user" | "agent";
  text: string;
  /** 思考摘要。原始思维链拿不到，这是模型自己的总结。 */
  thinking: string;
  tools: ToolCall[];
  /** 进度说明（如退避重试）。不是回复内容，不进对话历史。 */
  notices: string[];
};

type ChatStore = {
  turns: Turn[];
  busy: boolean;
  /** null = 还没问过 Rust。 */
  ready: boolean | null;
  unavailableReason: string | null;

  init: () => Promise<void>;
  send: (text: string) => Promise<void>;
  reset: () => Promise<void>;
};

/** 事件只订阅一次，与面板开关无关——关着面板也要收完这一轮。 */
let subscribed = false;

export const useChat = create<ChatStore>((set, get) => {
  function appendToLastAgentTurn(kind: "text" | "thinking" | "notice", text: string) {
    set((state) => {
      const turns = [...state.turns];
      const last = turns[turns.length - 1];
      const target: Turn =
        last?.role === "agent"
          ? { ...last }
          : { role: "agent", text: "", thinking: "", tools: [], notices: [] };
      if (kind === "thinking") target.thinking += text;
      else if (kind === "notice") target.notices = [...target.notices, text];
      else target.text += text;

      if (last?.role === "agent") turns[turns.length - 1] = target;
      else turns.push(target);
      return { turns };
    });
  }

  async function subscribe() {
    if (subscribed) return;
    subscribed = true;

    await listen<{ kind: string; text: string }>("agent://delta", (event) => {
      const { kind, text } = event.payload;
      // 工具走 agent://tool，那条信息更全。
      if (kind === "tool") return;
      if (kind === "thinking") appendToLastAgentTurn("thinking", text);
      else if (kind === "notice") appendToLastAgentTurn("notice", text);
      else appendToLastAgentTurn("text", text);
    });

    await listen<ToolCall>("agent://tool", (event) => {
      set((state) => {
        const turns = [...state.turns];
        const last = turns[turns.length - 1];
        if (last?.role !== "agent") return state;
        turns[turns.length - 1] = { ...last, tools: [...last.tools, event.payload] };
        return { turns };
      });
    });

    await listen("agent://layout-changed", () => {
      // agent 在 Rust 侧改了 ui.db，拉回来（并进 undo 栈）。
      void useEditor.getState().adoptExternalLayout();
    });

    await listen<{ ok: boolean; error: string | null }>("agent://done", (event) => {
      set({ busy: false });
      if (!event.payload.ok && event.payload.error) {
        appendToLastAgentTurn("text", `\n\n⚠️ ${event.payload.error}`);
      }
    });
  }

  return {
    turns: [],
    busy: false,
    ready: null,
    unavailableReason: null,

    async init() {
      await subscribe();
      const status = await ipc.agentStatus();
      set({ ready: status.ready, unavailableReason: status.reason });
    },

    async send(text) {
      if (get().busy || !get().ready) return;
      set((state) => ({
        busy: true,
        turns: [
          ...state.turns,
          { role: "user", text, thinking: "", tools: [], notices: [] },
          { role: "agent", text: "", thinking: "", tools: [], notices: [] },
        ],
      }));
      try {
        await ipc.agentSend(text);
      } catch {
        // 错误已经由 agent://done 呈现，这里不重复报。
        set({ busy: false });
      }
    },

    async reset() {
      await ipc.agentReset();
      set({ turns: [] });
    },
  };
});
