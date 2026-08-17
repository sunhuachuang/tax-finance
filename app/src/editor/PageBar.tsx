/** 页面切换。编辑模式下还能加页面、改名、删页面。 */
import { useEditor } from "../core/store";

export function PageBar() {
  const doc = useEditor((s) => s.doc);
  const pageId = useEditor((s) => s.pageId);
  const mode = useEditor((s) => s.mode);
  const setPage = useEditor((s) => s.setPage);
  const addPage = useEditor((s) => s.addPage);
  const renamePage = useEditor((s) => s.renamePage);
  const removePage = useEditor((s) => s.removePage);

  const editing = mode === "edit";

  return (
    <nav className="page-bar">
      {doc.pages.map((page) => {
        const active = page.id === pageId;
        return (
          <div key={page.id} className={`page-tab ${active ? "active" : ""}`}>
            {editing && active ? (
              // 选中的标签在编辑模式下直接就是输入框——改名不需要另开对话框。
              <input
                className="page-tab-input"
                value={page.title}
                aria-label="页面名称"
                onChange={(e) => renamePage(page.id, e.target.value)}
              />
            ) : (
              <button type="button" className="page-tab-button" onClick={() => setPage(page.id)}>
                {page.title || "未命名"}
              </button>
            )}

            {editing && active && doc.pages.length > 1 ? (
              <button
                type="button"
                className="page-tab-remove"
                title="删除这一页"
                onClick={() => removePage(page.id)}
              >
                ×
              </button>
            ) : null}
          </div>
        );
      })}

      {editing ? (
        <button type="button" className="page-add" onClick={addPage} title="新增页面">
          ＋
        </button>
      ) : null}
    </nav>
  );
}
