/**
 * 布局文档的状态与编辑历史。
 *
 * 所有修改都经 immer 的 `produceWithPatches`，于是 undo/redo 是白送的，
 * 而且 patch 格式正好就是将来 agent 改布局的接口格式：
 * 「帮我把 GST 那块挪到顶部」→ LLM 产出 patch → 走同一条路径落库。
 */
import { create } from "zustand";
import { applyPatches, enablePatches, produceWithPatches, type Patch } from "immer";

import { defaultLayout } from "./defaultLayout";
import * as ipc from "./ipc";
import { DATA_SOURCES } from "./ipc";
import { buildCatalog, instantiateBlock } from "./registry";
import {
  layoutDocSchema,
  resolveParamDefault,
  type Block,
  type Breakpoint,
  type LayoutDoc,
} from "./types";

enablePatches();

type HistoryStep = { patches: Patch[]; inverse: Patch[] };

type Status = "loading" | "ready" | "error";

type EditorStore = {
  doc: LayoutDoc;
  status: Status;
  error: string | null;
  /** 数据来源描述，状态栏用。 */
  source: string;

  pageId: string;
  mode: "view" | "edit";
  selectedId: string | null;
  breakpoint: Breakpoint;

  /**
   * 页面参数的当前取值，按 pageId 分组。
   *
   * 刻意**不进布局文档**：选一次申报期就写一版布局是荒谬的。
   * 参数的定义（有哪些、什么控件、默认值）在文档里，取值是会话状态。
   */
  paramValues: Record<string, Record<string, string>>;

  undoStack: HistoryStep[];
  redoStack: HistoryStep[];
  savedVersion: number | null;

  init: () => Promise<void>;
  /** agent 改完布局后从 ui.db 拉回来，进 undo 栈。 */
  adoptExternalLayout: () => Promise<void>;
  setMode: (mode: "view" | "edit") => void;
  setPage: (pageId: string) => void;
  setBreakpoint: (breakpoint: Breakpoint) => void;
  select: (id: string | null) => void;

  setParam: (pageId: string, key: string, value: string) => void;

  addBlock: (type: string) => void;
  removeBlock: (id: string) => void;
  moveBlock: (fromIndex: number, toIndex: number) => void;
  setSpan: (id: string, breakpoint: Breakpoint, span: number) => void;
  setProp: (id: string, key: string, value: unknown) => void;
  setCopy: (id: string, key: string, value: string) => void;

  addPage: () => void;
  renamePage: (pageId: string, title: string) => void;
  removePage: (pageId: string) => void;

  undo: () => void;
  redo: () => void;
  resetToDefault: () => void;
};

/** 自动保存的去抖延迟。编辑手感与写盘频率的折中。 */
const AUTOSAVE_MS = 600;
let autosaveTimer: ReturnType<typeof setTimeout> | undefined;

export const useEditor = create<EditorStore>((set, get) => {
  /** 所有文档修改的唯一入口：产出 patch、入历史栈、排一次自动保存。 */
  function mutate(recipe: (draft: LayoutDoc) => void) {
    const [next, patches, inverse] = produceWithPatches(get().doc, recipe);
    if (patches.length === 0) return;
    set((state) => ({
      doc: next,
      undoStack: [...state.undoStack, { patches, inverse }],
      // 新的修改会截断 redo 分支，和所有编辑器的行为一致。
      redoStack: [],
    }));
    scheduleSave();
  }

  function scheduleSave() {
    clearTimeout(autosaveTimer);
    autosaveTimer = setTimeout(() => {
      const doc = get().doc;
      ipc
        .saveLayout(doc)
        .then((version) => set({ savedVersion: version, error: null }))
        .catch((e: unknown) => set({ error: `保存布局失败：${String(e)}` }));
    }, AUTOSAVE_MS);
  }

  function currentPageIndex(doc: LayoutDoc, pageId: string) {
    const index = doc.pages.findIndex((p) => p.id === pageId);
    return index === -1 ? 0 : index;
  }

  return {
    doc: defaultLayout(),
    status: "loading",
    error: null,
    source: "",

    pageId: defaultLayout().pages[0].id,
    mode: "view",
    selectedId: null,
    breakpoint: "desktop",

    undoStack: [],
    redoStack: [],
    savedVersion: null,
    paramValues: seedParams(defaultLayout()),

    async init() {
      try {
        // agent 的白名单来自这份注册表。先推过去，否则 Rust 侧会拒绝一切布局写入
        // ——没有白名单就等于没有约束，那种情况下「通过」是错的。
        await ipc.setBlockCatalog(buildCatalog(Object.keys(DATA_SOURCES)));

        const [stored, source] = await Promise.all([ipc.loadLayout(), ipc.dataSource()]);
        // 存过的文档也要过一遍 schema：ui.db 可能是旧版本写的，
        // 校验不过就落回默认布局，而不是让渲染器面对一个畸形文档。
        const parsed = stored ? layoutDocSchema.safeParse(stored) : null;
        const doc = parsed?.success ? parsed.data : defaultLayout();

        // 没有存档时立刻把当前文档落一版。agent 在 Rust 侧读的是 ui.db，
        // 不落这一版它就看不到默认布局，只能报「布局还没初始化」。
        if (!parsed?.success) {
          void ipc.saveLayout(doc).catch(() => {});
        }

        set({
          doc,
          source,
          status: "ready",
          pageId: doc.pages[0].id,
          paramValues: seedParams(doc),
          error: parsed && !parsed.success ? "存档布局与当前版本不兼容，已回到默认布局" : null,
        });
      } catch (e) {
        set({ status: "error", error: `启动失败：${String(e)}` });
      }
    },

    /**
     * agent 在 Rust 侧改完布局后，把新文档拉回来。
     *
     * 走 `mutate` 而不是直接 set：这样 agent 的改动进 undo 栈，用户一个「撤销」
     * 就能退回去。代价是会多存一版（内容相同），换来的是 agent 的每一次改动
     * 都可回滚——这个交换很划算。
     */
    async adoptExternalLayout() {
      const stored = await ipc.loadLayout();
      const parsed = stored ? layoutDocSchema.safeParse(stored) : null;
      if (!parsed?.success) {
        set({ error: "agent 改出的布局无法解析，已忽略" });
        return;
      }
      const next = parsed.data;
      mutate((draft) => {
        draft.pages = next.pages;
      });
      set((state) => ({
        paramValues: { ...seedParams(next), ...state.paramValues },
        pageId: next.pages.some((p) => p.id === state.pageId)
          ? state.pageId
          : next.pages[0].id,
      }));
    },

    setParam(pageId, key, value) {
      set((state) => ({
        paramValues: {
          ...state.paramValues,
          [pageId]: { ...state.paramValues[pageId], [key]: value },
        },
      }));
    },

    setMode: (mode) => set({ mode, selectedId: mode === "view" ? null : get().selectedId }),
    setPage: (pageId) => set({ pageId, selectedId: null }),
    setBreakpoint: (breakpoint) => set({ breakpoint }),
    select: (id) => set({ selectedId: id }),

    addBlock(type) {
      const block = instantiateBlock(type);
      mutate((draft) => {
        draft.pages[currentPageIndex(draft, get().pageId)].blocks.push(block);
      });
      set({ selectedId: block.id });
    },

    removeBlock(id) {
      mutate((draft) => {
        const page = draft.pages[currentPageIndex(draft, get().pageId)];
        const index = page.blocks.findIndex((b) => b.id === id);
        if (index !== -1) page.blocks.splice(index, 1);
      });
      if (get().selectedId === id) set({ selectedId: null });
    },

    moveBlock(fromIndex, toIndex) {
      if (fromIndex === toIndex) return;
      mutate((draft) => {
        const blocks = draft.pages[currentPageIndex(draft, get().pageId)].blocks;
        const [moved] = blocks.splice(fromIndex, 1);
        if (moved) blocks.splice(toIndex, 0, moved);
      });
    },

    setSpan(id, breakpoint, span) {
      mutate((draft) => {
        const block = findBlock(draft, get().pageId, id);
        if (block) block.layout[breakpoint].span = span;
      });
    },

    setProp(id, key, value) {
      mutate((draft) => {
        const block = findBlock(draft, get().pageId, id);
        if (block) block.props[key] = value;
      });
    },

    setCopy(id, key, value) {
      mutate((draft) => {
        const block = findBlock(draft, get().pageId, id);
        if (!block) return;
        if (value === "") {
          // 清空即恢复默认文案，而不是存一个空字符串把标题抹掉。
          if (block.copy) delete block.copy[key];
        } else {
          block.copy ??= {};
          block.copy[key] = value;
        }
      });
    },

    undo() {
      const { undoStack, doc } = get();
      const step = undoStack.at(-1);
      if (!step) return;
      set((state) => ({
        doc: applyPatches(doc, step.inverse),
        undoStack: state.undoStack.slice(0, -1),
        redoStack: [...state.redoStack, step],
      }));
      scheduleSave();
    },

    redo() {
      const { redoStack, doc } = get();
      const step = redoStack.at(-1);
      if (!step) return;
      set((state) => ({
        doc: applyPatches(doc, step.patches),
        undoStack: [...state.undoStack, step],
        redoStack: state.redoStack.slice(0, -1),
      }));
      scheduleSave();
    },

    addPage() {
      const id = `page-${crypto.randomUUID().slice(0, 8)}`;
      mutate((draft) => {
        draft.pages.push({ id, title: "新页面", params: [], blocks: [] });
      });
      set({ pageId: id, selectedId: null });
    },

    renamePage(pageId, title) {
      mutate((draft) => {
        const page = draft.pages.find((p) => p.id === pageId);
        if (page) page.title = title;
      });
    },

    removePage(pageId) {
      // 文档 schema 要求至少一个页面，删到只剩一个就停手。
      if (get().doc.pages.length <= 1) return;
      mutate((draft) => {
        const index = draft.pages.findIndex((p) => p.id === pageId);
        if (index !== -1) draft.pages.splice(index, 1);
      });
      if (get().pageId === pageId) {
        set({ pageId: get().doc.pages[0].id, selectedId: null });
      }
    },

    resetToDefault() {
      const fresh = defaultLayout();
      mutate((draft) => {
        draft.pages = fresh.pages;
      });
      set({ pageId: fresh.pages[0].id, selectedId: null, paramValues: seedParams(fresh) });
    },
  };
});

function findBlock(doc: LayoutDoc, pageId: string, blockId: string): Block | undefined {
  const page = doc.pages.find((p) => p.id === pageId) ?? doc.pages[0];
  return page?.blocks.find((b) => b.id === blockId);
}

/** 每个页面参数的初始取值。 */
function seedParams(doc: LayoutDoc): Record<string, Record<string, string>> {
  const out: Record<string, Record<string, string>> = {};
  for (const page of doc.pages) {
    out[page.id] = Object.fromEntries(page.params.map((def) => [def.key, resolveParamDefault(def)]));
  }
  return out;
}

/**
 * 某页参数的当前取值。文档里新加的参数（存档里还没有取值）落到它的默认值，
 * 所以加一个参数不需要重启应用。
 */
export function usePageParams(pageId: string): Record<string, string> {
  const doc = useEditor((s) => s.doc);
  const values = useEditor((s) => s.paramValues[pageId]);
  const page = doc.pages.find((p) => p.id === pageId);
  if (!page) return {};
  return Object.fromEntries(
    page.params.map((def) => [def.key, values?.[def.key] ?? resolveParamDefault(def)]),
  );
}
