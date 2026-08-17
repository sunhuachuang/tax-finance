/**
 * 与 Rust shell 之间唯一的通道。
 *
 * 前端不知道数据是同进程的本地账本还是远端 host——那是 `backend` 层的事
 * （见 ARCHITECTURE.md）。这里只负责把 invoke 包成有类型的函数。
 */
import { invoke } from "@tauri-apps/api/core";

import type { LayoutDoc } from "./types";

/** 数据来源的一行描述，状态栏展示用。 */
export function dataSource(): Promise<string> {
  return invoke<string>("data_source");
}

export function overview(): Promise<unknown> {
  return invoke<unknown>("overview");
}

export function gst(params: { date?: string; frequency?: string } = {}): Promise<unknown> {
  return invoke<unknown>("gst", params);
}

export function ir3(year: string): Promise<unknown> {
  return invoke<unknown>("ir3", { year });
}

/**
 * 收一份文档。只造 pending 记录，不产生任何账。
 * 返回 `{ duplicate, document }`——同样的字节进来第二次是 duplicate。
 */
export function ingestDocument(path: string): Promise<{ duplicate: boolean; document: { id: string; original_filename?: string | null } }> {
  return invoke("ingest_document", { path });
}

/** 一份文档 + 记在它身上的所有 extraction。`local_path` 远程模式下为 null。 */
export function document(documentId: string): Promise<unknown> {
  return invoke<unknown>("document", { documentId });
}

/** 人工决定：`ignored` 或 `pending_extraction`。 */
export function setDocumentStatus(documentId: string, to: string): Promise<unknown> {
  return invoke<unknown>("set_document_status", { documentId, to });
}

/** 交给系统默认程序打开。只有本地模式可用。 */
export function openDocument(documentId: string): Promise<void> {
  return invoke<void>("open_document", { documentId });
}

/** 读设置。**不含 key 原文**——只有「有没有」、来源、和掩码提示。 */
export function getSettings(): Promise<unknown> {
  return invoke("get_settings");
}

/** 存 API key（空串 = 清除）。立刻生效，不用重启。 */
export function setApiKey(key: string): Promise<unknown> {
  return invoke("set_api_key", { key });
}

/** 存模型服务地址和模型名（都留空 = Anthropic 官方）。立刻生效。 */
export function setLlmEndpoint(baseUrl: string, model: string): Promise<unknown> {
  return invoke("set_llm_endpoint", { baseUrl, model });
}

/** 存远程 host（空串 = 回到本地模式）。要重启才生效。 */
export function setFinanceHost(host: string): Promise<unknown> {
  return invoke("set_finance_host", { host });
}

/** 把组件注册表推给 Rust —— agent 改布局时的白名单。 */
export function setBlockCatalog(catalog: unknown): Promise<void> {
  return invoke<void>("set_block_catalog", { catalog });
}

export function agentStatus(): Promise<{ ready: boolean; reason: string | null }> {
  return invoke("agent_status");
}

/** 发一句话。回复走 `agent://*` 事件流式回来，这个 Promise 在整轮结束时才 resolve。 */
export function agentSend(text: string): Promise<void> {
  return invoke<void>("agent_send", { text });
}

export function agentReset(): Promise<void> {
  return invoke<void>("agent_reset");
}

export function agentSetEffort(effort: string): Promise<void> {
  return invoke<void>("agent_set_effort", { effort });
}

/** 人工确认闸口。只应该由用户点击触发。 */
export function approveEntry(entryId: string): Promise<unknown> {
  return invoke<unknown>("approve_entry", { entryId });
}

export function rejectEntry(entryId: string): Promise<unknown> {
  return invoke<unknown>("reject_entry", { entryId });
}

export function loadLayout(): Promise<unknown | null> {
  return invoke<unknown | null>("load_layout");
}

/** 返回新的版本号。每次保存追加一版，不覆盖。 */
export function saveLayout(doc: LayoutDoc): Promise<number> {
  return invoke<number>("save_layout", { doc });
}

/**
 * 数据块能引用的来源。binding 只能指向这里列出的名字——
 * 布局文档不是任意 IPC 调用的入口。
 *
 * 每个来源都接一个字符串字典：参数从页面参数来（binding 里写 `$date`），
 * 缺省由后端决定，前端不替它编默认值。
 */
export const DATA_SOURCES = {
  overview: () => overview(),
  gst: (params: Record<string, string>) =>
    gst({ date: params.date, frequency: params.frequency }),
  ir3: (params: Record<string, string>) => ir3(params.year ?? ""),
  // 没选中任何文档时不打后端：空 id 只会换回一条「不是合法 UUID」的报错，
  // 而正确的界面状态是「还没选」，不是「出错了」。
  document: (params: Record<string, string>) =>
    params.id ? document(params.id) : Promise.resolve(null),
} satisfies Record<string, (params: Record<string, string>) => Promise<unknown>>;

export type DataSourceName = keyof typeof DATA_SOURCES;

export function isDataSource(name: string): name is DataSourceName {
  return Object.hasOwn(DATA_SOURCES, name);
}
