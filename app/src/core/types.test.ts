/**
 * 布局文档层的纯函数测试。
 *
 * 重点是 `resolvePath`：数据块的数字是它取出来的，它取错了整屏数字就是错的，
 * 而且错得没有任何提示。
 */
import { describe, expect, it } from "vitest";

import {
  currentTaxYearEnd,
  resolveBindingParams,
  resolvePath,
  sourceKey,
  taxYearLabel,
  type Binding,
} from "./types";

const gstReturn = {
  period: { start: "2026-02-01", end: "2026-03-31" },
  lines: [
    { code: "gst101.box10", label: "销项", amount: { cents: 3000, currency: "NZD" } },
    { code: "gst101.box15", label: "应缴", amount: { cents: 600, currency: "NZD" } },
  ],
};

describe("resolvePath", () => {
  it("空路径返回整个对象", () => {
    expect(resolvePath(gstReturn, "")).toBe(gstReturn);
  });

  it("走普通的点分键", () => {
    expect(resolvePath(gstReturn, "period.end")).toBe("2026-03-31");
  });

  it("走数组下标", () => {
    expect(resolvePath(gstReturn, "lines.0.code")).toBe("gst101.box10");
  });

  it("按字段值在数组里选一项", () => {
    expect(resolvePath(gstReturn, "lines[code=gst101.box15].amount")).toEqual({
      cents: 600,
      currency: "NZD",
    });
  });

  it("选择器里的点不会被当成路径分隔符", () => {
    // "gst101.box10" 自己带点，按点切分会把它拆坏。
    expect(resolvePath(gstReturn, "lines[code=gst101.box10].label")).toBe("销项");
  });

  it("选不中就是 undefined，不抛异常也不回退到第一项", () => {
    expect(resolvePath(gstReturn, "lines[code=gst101.box99].amount")).toBeUndefined();
  });

  it("中途断掉返回 undefined", () => {
    expect(resolvePath(gstReturn, "nope.deeper.still")).toBeUndefined();
    expect(resolvePath(null, "a.b")).toBeUndefined();
  });
});

describe("resolveBindingParams", () => {
  const binding = (params: Record<string, string>): Binding => ({
    source: "gst",
    path: "",
    agg: "value",
    params,
  });

  it("$ 开头引用页面参数", () => {
    expect(resolveBindingParams(binding({ date: "$date" }), { date: "2026-02-14" })).toEqual({
      date: "2026-02-14",
    });
  });

  it("不带 $ 的是字面量", () => {
    expect(resolveBindingParams(binding({ frequency: "monthly" }), {})).toEqual({
      frequency: "monthly",
    });
  });

  it("引用不到的参数留空串，交给后端决定缺省", () => {
    expect(resolveBindingParams(binding({ date: "$missing" }), {})).toEqual({ date: "" });
  });
});

describe("sourceKey", () => {
  it("无参数就是来源名本身", () => {
    expect(sourceKey("overview", {})).toBe("overview");
  });

  it("参数顺序不影响缓存键", () => {
    expect(sourceKey("gst", { date: "2026-02-14", frequency: "monthly" })).toBe(
      sourceKey("gst", { frequency: "monthly", date: "2026-02-14" }),
    );
  });

  it("不同参数是不同的键", () => {
    expect(sourceKey("gst", { date: "2026-02-14" })).not.toBe(sourceKey("gst", { date: "2026-05-14" }));
  });
});

describe("税年", () => {
  it("标签与 taxcore::TaxYear::label 一致", () => {
    expect(taxYearLabel(2026)).toBe("2025-26");
    expect(taxYearLabel(2030)).toBe("2029-30");
    // 世纪边界上后两位要补零，不能变成 "2099-0"。
    expect(taxYearLabel(2100)).toBe("2099-00");
  });

  it("4 月 1 日进入下一个税年", () => {
    expect(currentTaxYearEnd(new Date(2026, 2, 31))).toBe(2026);
    expect(currentTaxYearEnd(new Date(2026, 3, 1))).toBe(2027);
  });
});
