/**
 * 属性面板。完全由注册表里的 `fields` / `copy` 生成——
 * 新增块型不需要碰这个文件。
 *
 * 注意这里能改什么：位置、宽度、文案、块自己的配置。**不能改数字**。
 * 数据块的值只出现在只读的「数据来源」一栏（ARCHITECTURE.md 约束 2）。
 */
import { KIND_LABELS, getBlockDef, type FieldDef } from "../core/registry";
import { useEditor } from "../core/store";
import { DESKTOP_COLUMNS, MOBILE_COLUMNS } from "../core/types";

export function Inspector() {
  const doc = useEditor((s) => s.doc);
  const pageId = useEditor((s) => s.pageId);
  const selectedId = useEditor((s) => s.selectedId);
  const setSpan = useEditor((s) => s.setSpan);
  const setProp = useEditor((s) => s.setProp);
  const setCopy = useEditor((s) => s.setCopy);
  const removeBlock = useEditor((s) => s.removeBlock);

  const page = doc.pages.find((p) => p.id === pageId) ?? doc.pages[0];
  const block = page.blocks.find((b) => b.id === selectedId);

  if (!block) {
    return (
      <aside className="inspector">
        <div className="panel-title">属性</div>
        <p className="panel-hint">选中一个块来编辑它。拖动块头部的把手可以重排。</p>
      </aside>
    );
  }

  const def = getBlockDef(block.type);
  if (!def) {
    return (
      <aside className="inspector">
        <div className="panel-title">属性</div>
        <p className="panel-hint">未注册的块型 {block.type}，无法编辑。</p>
      </aside>
    );
  }

  return (
    <aside className="inspector">
      <div className="panel-title">
        {def.name}
        <span className={`kind-tag kind-${def.kind}`}>
          {KIND_LABELS[def.kind]}
        </span>
      </div>

      <section className="panel-section">
        <div className="panel-label">宽度（栅格列数）</div>
        <SpanSlider
          label="桌面"
          value={block.layout.desktop.span}
          max={DESKTOP_COLUMNS}
          onChange={(v) => setSpan(block.id, "desktop", v)}
        />
        <SpanSlider
          label="移动"
          value={block.layout.mobile.span}
          max={MOBILE_COLUMNS}
          onChange={(v) => setSpan(block.id, "mobile", v)}
        />
      </section>

      {def.copy?.length ? (
        <section className="panel-section">
          <div className="panel-label">文案</div>
          {def.copy.map((slot) => (
            <label key={slot.key} className="field">
              <span className="field-label">{slot.label}</span>
              <input
                type="text"
                value={block.copy?.[slot.key] ?? ""}
                placeholder={slot.fallback}
                onChange={(e) => setCopy(block.id, slot.key, e.target.value)}
              />
            </label>
          ))}
          <p className="panel-hint">留空恢复默认文案。</p>
        </section>
      ) : null}

      {def.fields?.length ? (
        <section className="panel-section">
          <div className="panel-label">配置</div>
          {def.fields.map((field) => (
            <Field
              key={field.key}
              field={field}
              value={block.props[field.key]}
              onChange={(v) => setProp(block.id, field.key, v)}
            />
          ))}
        </section>
      ) : null}

      {block.binding ? (
        <section className="panel-section">
          <div className="panel-label">数据来源</div>
          {/* 只读。值来自账本查询，不给手填的入口。 */}
          <div className="binding-readout">
            <code>
              {block.binding.source}.{block.binding.path || "*"}
            </code>
            {block.binding.agg === "count" ? <span className="binding-agg">计数</span> : null}
          </div>
          <p className="panel-hint">数字只能来自账本查询，不能手填。</p>
        </section>
      ) : null}

      <section className="panel-section">
        {def.locked ? (
          <p className="panel-hint locked-hint">
            这是人工确认闸口，锁定，不可删除。
          </p>
        ) : (
          <button type="button" className="btn danger" onClick={() => removeBlock(block.id)}>
            删除这个块
          </button>
        )}
      </section>
    </aside>
  );
}

function SpanSlider({
  label,
  value,
  max,
  onChange,
}: {
  label: string;
  value: number;
  max: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="field">
      <span className="field-label">
        {label}
        <span className="field-value">
          {value}/{max}
        </span>
      </span>
      <input
        type="range"
        min={1}
        max={max}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </label>
  );
}

function Field({
  field,
  value,
  onChange,
}: {
  field: FieldDef;
  value: unknown;
  onChange: (value: unknown) => void;
}) {
  switch (field.control) {
    case "text":
      return (
        <label className="field">
          <span className="field-label">{field.label}</span>
          <input
            type="text"
            value={typeof value === "string" ? value : ""}
            placeholder={field.placeholder}
            onChange={(e) => onChange(e.target.value)}
          />
        </label>
      );
    case "number":
      return (
        <label className="field">
          <span className="field-label">{field.label}</span>
          <input
            type="number"
            min={field.min}
            max={field.max}
            value={typeof value === "number" ? value : ""}
            onChange={(e) => onChange(Number(e.target.value))}
          />
        </label>
      );
    case "toggle":
      return (
        <label className="field field-inline">
          <input type="checkbox" checked={value === true} onChange={(e) => onChange(e.target.checked)} />
          <span className="field-label">{field.label}</span>
        </label>
      );
    case "select":
      return (
        <label className="field">
          <span className="field-label">{field.label}</span>
          <select value={typeof value === "string" ? value : ""} onChange={(e) => onChange(e.target.value)}>
            {field.options.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
      );
  }
}
