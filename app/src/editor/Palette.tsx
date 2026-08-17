/**
 * 块面板：可以往页面上加什么。
 *
 * 列表直接来自注册表，所以它天然就是白名单——用户只能加这里有的东西。
 */
import { KIND_LABELS, allBlockDefs } from "../core/registry";
import { useEditor } from "../core/store";

export function Palette() {
  const addBlock = useEditor((s) => s.addBlock);

  return (
    <section className="panel-section palette">
      <div className="panel-label">添加块</div>
      {allBlockDefs().map((def) => (
        <button
          key={def.type}
          type="button"
          className="palette-item"
          onClick={() => addBlock(def.type)}
        >
          <span className="palette-name">
            {def.name}
            <span className={`kind-tag kind-${def.kind}`}>
              {KIND_LABELS[def.kind]}
            </span>
          </span>
          <span className="palette-hint">{def.hint}</span>
        </button>
      ))}
    </section>
  );
}
