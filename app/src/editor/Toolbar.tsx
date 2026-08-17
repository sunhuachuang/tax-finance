/** 顶栏：模式切换、撤销重做、断点预览、刷新数据。 */
import { useData } from "../core/data";
import { useEditor } from "../core/store";
import { useSettings } from "../settings/store";

export function Toolbar({
  chatOpen,
  onToggleChat,
}: {
  chatOpen: boolean;
  onToggleChat: () => void;
}) {
  const mode = useEditor((s) => s.mode);
  const setMode = useEditor((s) => s.setMode);
  const breakpoint = useEditor((s) => s.breakpoint);
  const setBreakpoint = useEditor((s) => s.setBreakpoint);
  const undo = useEditor((s) => s.undo);
  const redo = useEditor((s) => s.redo);
  const canUndo = useEditor((s) => s.undoStack.length > 0);
  const canRedo = useEditor((s) => s.redoStack.length > 0);
  const resetToDefault = useEditor((s) => s.resetToDefault);
  const data = useData();
  const showSettings = useSettings((s) => s.show);

  return (
    <header className="toolbar">
      <div className="toolbar-group">
        <strong className="brand">Finance</strong>
      </div>

      <div className="toolbar-group">
        {/* 对话在前，手动编辑在后——这是现在的主次关系。 */}
        <button
          type="button"
          className={`btn ${chatOpen && mode !== "edit" ? "active" : ""}`}
          onClick={onToggleChat}
        >
          AI 助手
        </button>
        <button
          type="button"
          className={`btn ${mode === "edit" ? "active" : ""}`}
          onClick={() => setMode(mode === "edit" ? "view" : "edit")}
          title="直接拖拽和调属性。agent 走的是同一份布局文档和同一个撤销栈。"
        >
          {mode === "edit" ? "完成编辑" : "手动编辑"}
        </button>

        {mode === "edit" ? (
          <>
            <button type="button" className="btn" disabled={!canUndo} onClick={undo}>
              撤销
            </button>
            <button type="button" className="btn" disabled={!canRedo} onClick={redo}>
              重做
            </button>
            <button type="button" className="btn subtle" onClick={resetToDefault}>
              恢复默认布局
            </button>
          </>
        ) : null}
      </div>

      <div className="toolbar-group">
        {/* 桌面上预览移动断点：同一份文档，两套 span。 */}
        <div className="segmented">
          <button
            type="button"
            className={breakpoint === "desktop" ? "active" : ""}
            onClick={() => setBreakpoint("desktop")}
          >
            桌面
          </button>
          <button
            type="button"
            className={breakpoint === "mobile" ? "active" : ""}
            onClick={() => setBreakpoint("mobile")}
          >
            移动
          </button>
        </div>
        <button type="button" className="btn" onClick={data.refreshAll}>
          刷新数据
        </button>
        <button type="button" className="btn" onClick={() => void showSettings()}>
          设置
        </button>
      </div>
    </header>
  );
}
