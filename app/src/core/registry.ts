/**
 * 组件注册表 —— 唯一的扩展点。
 *
 * 渲染器、编辑器属性面板、校验、序列化全部从这一份定义派生，
 * 所以新增一个块型只需要在这里注册一次。
 *
 * 同时它是「布局是数据，不是代码」的执行点：布局文档里的 `type` 只能命中
 * 这张表，命不中就渲染占位错误块。用户无法通过布局引入任何可执行的东西。
 */
import type { ComponentType } from "react";
import type { ZodType } from "zod";

import type { Binding, Block } from "./types";

/**
 * 块的类别，决定它在 UI 上如何呈现，也决定它被允许做什么。
 *
 * - `data` 值来自 binding，不可手改，必须标注来源。
 * - `text` 自由文案，样式上明确表现为注释/标签，不冒充数据。
 * - `action` 会写入的动作块（如收文档），只造 pending，不产生账。
 * - `gate` 人工确认闸口，锁定，不可删除。
 *
 * 分类不是装饰：写操作和读展示在视觉上必须能一眼分开。
 */
export type BlockKind = "data" | "text" | "action" | "gate";

/** 类别在界面上的名字。块面板和属性面板共用一份，免得两处说法不一致。 */
export const KIND_LABELS: Record<BlockKind, string> = {
  data: "数据块",
  text: "文本块",
  action: "动作块",
  gate: "闸口",
};

export type FieldDef =
  | { key: string; label: string; control: "text"; placeholder?: string }
  | { key: string; label: string; control: "number"; min?: number; max?: number }
  | { key: string; label: string; control: "toggle" }
  | {
      key: string;
      label: string;
      control: "select";
      options: { value: string; label: string }[];
    };

/** 可被用户改写的文案位。 */
export type CopySlot = { key: string; label: string; fallback: string };

export type BlockViewProps = {
  block: Block;
  /** binding 解析出的值。没有 binding 或数据未就绪时为 undefined。 */
  value: unknown;
  /** 取文案：先看用户覆写，再落到默认。 */
  text: (key: string) => string;
  /** 数据源是否还在加载。 */
  loading: boolean;
  /** 数据源的错误信息。 */
  error: string | null;
  /** 当前是否在编辑模式——编辑态下交互块应停用真实动作。 */
  editing: boolean;
  /** 请求刷新该块的数据源。 */
  refresh: () => void;
  /** 当前页面参数的取值。 */
  params: Record<string, string>;
  /**
   * 写一个页面参数。主从联动就靠这个：列表块写 `document`，
   * 详情块 binding 里写 `$document`，两个块不需要互相知道。
   */
  setParam: (key: string, value: string) => void;
};

export type BlockDef = {
  type: string;
  /** 编辑器里显示的名字。 */
  name: string;
  /** 一句话说明，块面板里展示。 */
  hint: string;
  kind: BlockKind;
  /** 锁定块不可删除。确认闸口必须锁定。 */
  locked?: boolean;
  propsSchema: ZodType;
  defaultProps: Record<string, unknown>;
  defaultBinding?: Binding;
  defaultSpan: { desktop: number; mobile: number };
  /** 属性面板要渲染的字段。 */
  fields?: FieldDef[];
  /** 该块暴露给用户改写的文案位。 */
  copy?: CopySlot[];
  Component: ComponentType<BlockViewProps>;
};

const registry = new Map<string, BlockDef>();

export function registerBlock(def: BlockDef): void {
  if (registry.has(def.type)) {
    throw new Error(`块型 ${def.type} 重复注册`);
  }
  registry.set(def.type, def);
}

export function getBlockDef(type: string): BlockDef | undefined {
  return registry.get(type);
}

export function allBlockDefs(): BlockDef[] {
  return [...registry.values()];
}

/**
 * 按注册表的默认值造一个新块。编辑器里从块面板拖出来时走这条路径，
 * 保证任何新块从诞生起就满足 schema。
 */
export function instantiateBlock(type: string): Block {
  const def = registry.get(type);
  if (!def) throw new Error(`未注册的块型 ${type}`);
  return {
    id: `${type}-${crypto.randomUUID().slice(0, 8)}`,
    type: def.type,
    layout: {
      desktop: { span: def.defaultSpan.desktop },
      mobile: { span: def.defaultSpan.mobile },
    },
    props: structuredClone(def.defaultProps),
    binding: def.defaultBinding ? structuredClone(def.defaultBinding) : undefined,
  };
}

/**
 * 把注册表导出成 Rust 侧能用的白名单。
 *
 * agent 改布局的合法性边界就是这份数据。**它必须从注册表生成，不能在
 * Rust 里另抄一份**——两处定义迟早漂移，而漂移的结果是 agent 生成出
 * 渲染器认不出的块。
 */
export function buildCatalog(sources: string[]) {
  return {
    blocks: allBlockDefs().map((def) => ({
      type: def.type,
      name: def.name,
      hint: def.hint,
      kind: def.kind,
      locked: def.locked ?? false,
      prop_keys: def.fields?.map((f) => f.key) ?? [],
      copy_keys: def.copy?.map((c) => c.key) ?? [],
      default_binding: def.defaultBinding ?? null,
    })),
    sources,
  };
}
