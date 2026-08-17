/**
 * 出厂布局。用户没存过任何版本、或存档与当前 schema 不兼容时落到这里。
 *
 * 它本身也只是一份普通的布局文档——没有任何「内置页面」的特权，
 * 用户可以把它改得面目全非、加页面、删页面。唯一的例外是 review-gate：
 * 注册表把它标成 locked，编辑器不给删。
 */
import type { Block, LayoutDoc, Page } from "./types";

export function defaultLayout(): LayoutDoc {
  return {
    version: 1,
    pages: [overviewPage(), documentsPage(), gstPage(), incomeTaxPage()],
  };
}

function overviewPage(): Page {
  return {
    id: "home",
    title: "总览",
    params: [],
    blocks: [
      stat("stat-drafts", "待审草稿", "review_drafts", " 笔", "attention"),
      stat("stat-docs", "待处理文档", "review_documents", " 份", "neutral"),
      stat("stat-posted", "已入账分录", "posted_entries", " 笔", "neutral"),
      stat("stat-bank", "未对账流水", "unreconciled_bank", " 条", "attention"),
      {
        id: "intake-main",
        type: "document-intake",
        layout: { desktop: { span: 12 }, mobile: { span: 4 } },
        props: { acceptDrop: true },
      },
      {
        id: "gate-main",
        type: "review-gate",
        layout: { desktop: { span: 12 }, mobile: { span: 4 } },
        props: { maxRows: 8 },
        binding: { source: "overview", path: "review_drafts", agg: "value" },
      },
      {
        id: "table-entries",
        type: "record-table",
        layout: { desktop: { span: 12 }, mobile: { span: 4 } },
        props: { columns: "date,narration,status", maxRows: 10 },
        copy: { title: "已入账分录", empty: "账本还是空的。" },
        binding: { source: "overview", path: "posted_entries", agg: "value" },
      },
    ],
  };
}

function documentsPage(): Page {
  return {
    id: "documents",
    title: "文档",
    // hidden：由列表块点选时写入，不给人一个粘 UUID 的输入框。
    params: [{ key: "document", label: "选中的文档", control: "hidden", default: "" }],
    blocks: [
      {
        id: "doc-intake",
        type: "document-intake",
        layout: { desktop: { span: 12 }, mobile: { span: 4 } },
        props: { acceptDrop: true },
      },
      {
        id: "doc-list",
        type: "document-list",
        layout: { desktop: { span: 5 }, mobile: { span: 4 } },
        props: { paramKey: "document", status: "all", maxRows: 15 },
        copy: { title: "文档", empty: "还没有文档，把发票拖进来。" },
        binding: { source: "overview", path: "documents", agg: "value" },
      },
      {
        id: "doc-detail",
        type: "document-detail",
        layout: { desktop: { span: 7 }, mobile: { span: 4 } },
        props: { showRawPayload: false },
        copy: { title: "文档详情", empty: "在左边选一份文档。" },
        binding: { source: "document", path: "", agg: "value", params: { id: "$document" } },
      },
    ],
  };
}

function gstPage(): Page {
  return {
    id: "gst",
    title: "GST",
    // 选一天，引擎算出这天落在哪个申报期。
    params: [
      { key: "date", label: "申报期内任一天", control: "date", default: "today" },
    ],
    blocks: [
      {
        id: "gst-payable",
        type: "stat",
        layout: { desktop: { span: 4 }, mobile: { span: 4 } },
        props: { suffix: "", tone: "attention" },
        copy: { title: "应缴 GST（正数缴税 / 负数退税）" },
        binding: {
          source: "gst",
          // 按 code 定位而不是下标：引擎调整行序时不会静默指向另一个数字。
          path: "lines[code=gst101.box15].amount",
          agg: "value",
          params: { date: "$date" },
        },
      },
      {
        id: "gst-collected",
        type: "stat",
        layout: { desktop: { span: 4 }, mobile: { span: 2 } },
        props: { suffix: "", tone: "neutral" },
        copy: { title: "销项 GST（box 10）" },
        binding: {
          source: "gst",
          path: "lines[code=gst101.box10].amount",
          agg: "value",
          params: { date: "$date" },
        },
      },
      {
        id: "gst-credit",
        type: "stat",
        layout: { desktop: { span: 4 }, mobile: { span: 2 } },
        props: { suffix: "", tone: "neutral" },
        copy: { title: "进项抵扣（box 14）" },
        binding: {
          source: "gst",
          path: "lines[code=gst101.box14].amount",
          agg: "value",
          params: { date: "$date" },
        },
      },
      {
        id: "gst-return",
        type: "tax-return",
        layout: { desktop: { span: 12 }, mobile: { span: 4 } },
        props: { expandAll: false },
        copy: { title: "GST101", empty: "这一期没有已入账的分录。" },
        binding: { source: "gst", path: "", agg: "value", params: { date: "$date" } },
      },
    ],
  };
}

function incomeTaxPage(): Page {
  return {
    id: "income-tax",
    title: "所得税",
    params: [
      { key: "year", label: "税年", control: "tax-year", default: "current-tax-year" },
    ],
    blocks: [
      {
        id: "ir3-profit",
        type: "stat",
        layout: { desktop: { span: 4 }, mobile: { span: 4 } },
        props: { suffix: "", tone: "attention" },
        copy: { title: "净利润" },
        binding: {
          source: "ir3",
          path: "lines[code=ir3.net_profit].amount",
          agg: "value",
          params: { year: "$year" },
        },
      },
      {
        id: "ir3-income",
        type: "stat",
        layout: { desktop: { span: 4 }, mobile: { span: 2 } },
        props: { suffix: "", tone: "neutral" },
        copy: { title: "业务收入（不含 GST）" },
        binding: {
          source: "ir3",
          path: "lines[code=ir3.income].amount",
          agg: "value",
          params: { year: "$year" },
        },
      },
      {
        id: "ir3-expenses",
        type: "stat",
        layout: { desktop: { span: 4 }, mobile: { span: 2 } },
        props: { suffix: "", tone: "neutral" },
        copy: { title: "业务支出" },
        binding: {
          source: "ir3",
          path: "lines[code=ir3.expenses].amount",
          agg: "value",
          params: { year: "$year" },
        },
      },
      {
        id: "ir3-return",
        type: "tax-return",
        layout: { desktop: { span: 12 }, mobile: { span: 4 } },
        props: { expandAll: false },
        copy: { title: "IR3 汇总", empty: "这一税年没有已入账的分录。" },
        binding: { source: "ir3", path: "", agg: "value", params: { year: "$year" } },
      },
      {
        id: "ir3-scope-note",
        type: "note",
        layout: { desktop: { span: 12 }, mobile: { span: 4 } },
        props: { level: "note" },
        copy: {
          body: "IR3 尚未建模法定扣除调整（娱乐 50%、home office、里程）。上面表格里的 note 会列出具体缺什么。",
        },
      },
    ],
  };
}

/** 总览页那四个计数卡的样板，避免重复。 */
function stat(id: string, title: string, path: string, suffix: string, tone: string): Block {
  return {
    id,
    type: "stat",
    layout: { desktop: { span: 3 }, mobile: { span: 2 } },
    props: { suffix, tone },
    copy: { title },
    binding: { source: "overview", path, agg: "count" },
  };
}
