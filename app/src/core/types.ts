/**
 * 布局文档 —— 界面的唯一真相源。
 *
 * 界面不是组件树，是这份 JSON：渲染器读它，编辑器改它，agent 未来也只改它。
 * 见 ARCHITECTURE.md「UI as Data」。
 */
import { z } from "zod";

/** 12 栅格。桌面 1..12，移动 1..4（移动端栅格更窄）。 */
export const DESKTOP_COLUMNS = 12;
export const MOBILE_COLUMNS = 4;

/**
 * 数据块的取值来源。数字只能从这里来，不能手打——
 * 这是「可定制的是呈现，不是数字」在类型层的落点。
 */
export const bindingSchema = z.object({
  /** 后端命令名，如 "overview"。 */
  source: z.string().min(1),
  /** 结果 JSON 里的点分路径，如 "review_drafts"。空串表示整个结果。 */
  path: z.string(),
  /** 取到值之后怎么用：原样 / 数组计数。 */
  agg: z.enum(["value", "count"]).default("value"),
  /**
   * 传给数据源的参数，如 GST 的 `{ date }`、IR3 的 `{ year }`。
   * 值以 `$` 开头表示引用页面参数：`{ date: "$date" }`。
   */
  params: z.record(z.string(), z.string()).optional(),
});
export type Binding = z.infer<typeof bindingSchema>;

/**
 * 页面级参数。一个 GST 页面上的六个块看的是同一个申报期，
 * 所以「看哪一期」属于页面，不属于每个块。
 *
 * 定义在布局文档里（用户可以加页面、加参数），当前**取值**不在文档里
 * ——那是会话状态，不该每选一次日期就写一版布局。
 */
export const paramDefSchema = z.object({
  key: z.string().min(1),
  label: z.string(),
  /** `hidden` 的参数不出现在参数条里，由块自己写（如列表选中了哪一行）。 */
  control: z.enum(["date", "tax-year", "text", "hidden"]),
  /** 初始值。`today` 表示今天，`current-tax-year` 表示当前税年。 */
  default: z.string(),
});
export type ParamDef = z.infer<typeof paramDefSchema>;

export const blockLayoutSchema = z.object({
  desktop: z.object({ span: z.number().int().min(1).max(DESKTOP_COLUMNS) }),
  mobile: z.object({ span: z.number().int().min(1).max(MOBILE_COLUMNS) }),
});
export type BlockLayout = z.infer<typeof blockLayoutSchema>;

/**
 * 一个块。`type` 只能取自组件注册表白名单——渲染器遇到未注册的 type
 * 显示占位错误块，而不是尝试执行任何东西。
 *
 * 块在页面上的顺序 = 数组顺序。拖拽重排改的就是这个数组。
 */
export const blockSchema = z.object({
  id: z.string().min(1),
  type: z.string().min(1),
  layout: blockLayoutSchema,
  /** 该块自己的配置，由块定义的 propsSchema 二次校验。 */
  props: z.record(z.string(), z.unknown()).default({}),
  /** 文案覆写。用户改标题改的是这里，不动代码里的默认文案。 */
  copy: z.record(z.string(), z.string()).optional(),
  binding: bindingSchema.optional(),
});
export type Block = z.infer<typeof blockSchema>;

export const pageSchema = z.object({
  id: z.string().min(1),
  title: z.string(),
  params: z.array(paramDefSchema).default([]),
  blocks: z.array(blockSchema).default([]),
});
export type Page = z.infer<typeof pageSchema>;

export const layoutDocSchema = z.object({
  /** 文档结构的版本，用于将来做迁移。不是保存次数。 */
  version: z.literal(1),
  pages: z.array(pageSchema).min(1),
});
export type LayoutDoc = z.infer<typeof layoutDocSchema>;

/** 当前渲染断点。同一份文档、两套 span。 */
export type Breakpoint = "desktop" | "mobile";

/**
 * 路径取值。任何一段取不到都返回 undefined，不抛异常。
 *
 * 支持两种段：
 * - `review_drafts`、`0` —— 普通键或数组下标
 * - `[code=gst101.box15]` —— 在数组里按字段值找一项
 *
 * 后者的存在理由是：申报表的行是按 `code` 标识的，用下标绑定
 * （`lines.10.amount`）会在引擎调整行序时静默指向另一个数字。
 */
const PATH_TOKEN = /\[([^\]]+)\]|([^.[\]]+)/g;

export function resolvePath(root: unknown, path: string): unknown {
  if (!path) return root;

  let acc: unknown = root;
  for (const match of path.matchAll(PATH_TOKEN)) {
    if (acc === null || acc === undefined) return undefined;

    const [, selector, key] = match;
    if (selector !== undefined) {
      const [field, ...rest] = selector.split("=");
      const wanted = rest.join("=");
      if (!Array.isArray(acc)) return undefined;
      acc = acc.find((item) => (item as Record<string, unknown>)?.[field] === wanted);
    } else {
      acc = (acc as Record<string, unknown>)[key];
    }
  }
  return acc;
}

/** 把 binding 落到一个具体的值。 */
export function resolveBinding(root: unknown, binding: Binding): unknown {
  const value = resolvePath(root, binding.path);
  if (binding.agg === "count") {
    return Array.isArray(value) ? value.length : 0;
  }
  return value;
}

/** `YYYY-MM-DD`，本地时区。 */
export function today(): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/**
 * NZ 税年标签，与 `taxcore::TaxYear::label` 一致：截至 3 月 31 日的年份 Y
 * 标作 `${Y-1}-${Y 的后两位}`。4 月起进入下一个税年。
 */
export function taxYearLabel(endingYear: number): string {
  return `${endingYear - 1}-${String(endingYear % 100).padStart(2, "0")}`;
}

export function currentTaxYearEnd(date = new Date()): number {
  return date.getMonth() + 1 >= 4 ? date.getFullYear() + 1 : date.getFullYear();
}

/** 参数的初始值。`today` / `current-tax-year` 是两个约定的占位符。 */
export function resolveParamDefault(def: ParamDef): string {
  if (def.default === "today") return today();
  if (def.default === "current-tax-year") return taxYearLabel(currentTaxYearEnd());
  return def.default;
}

/**
 * 把 binding 里 `$key` 形式的引用换成页面参数的当前值。
 * 引用不到的参数留空串——由数据源自己决定缺省行为。
 */
export function resolveBindingParams(
  binding: Binding,
  pageParams: Record<string, string>,
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [key, raw] of Object.entries(binding.params ?? {})) {
    out[key] = raw.startsWith("$") ? (pageParams[raw.slice(1)] ?? "") : raw;
  }
  return out;
}

/** 数据缓存的键。同一个来源不同参数是两份缓存。 */
export function sourceKey(source: string, params: Record<string, string>): string {
  const entries = Object.entries(params).sort(([a], [b]) => a.localeCompare(b));
  return entries.length === 0 ? source : `${source}?${entries.map(([k, v]) => `${k}=${v}`).join("&")}`;
}
