/** 值的显示格式化。只做展示，不做任何算术——数字一律原样来自后端。 */

/** `taxcore::Money` 的序列化形状。 */
type Money = { cents: number; currency: string };

function isMoney(value: unknown): value is Money {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as Money).cents === "number" &&
    typeof (value as Money).currency === "string"
  );
}

/** 分转成带两位小数的金额串。除以 100 是进制转换，不是计算。 */
export function formatMoney({ cents, currency }: Money): string {
  const sign = cents < 0 ? "-" : "";
  const abs = Math.abs(cents);
  const major = Math.trunc(abs / 100);
  const minor = String(abs % 100).padStart(2, "0");
  return `${sign}${major.toLocaleString("en-NZ")}.${minor} ${currency}`;
}

/** 任意值的单元格显示。对象兜底为紧凑 JSON，好过显示 [object Object]。 */
export function formatValue(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (isMoney(value)) return formatMoney(value);
  if (typeof value === "boolean") return value ? "是" : "否";
  if (typeof value === "number" || typeof value === "string") return String(value);
  if (Array.isArray(value)) return `${value.length} 项`;
  return JSON.stringify(value);
}
