/**
 * 页面参数条：这一页在看哪个申报期 / 哪个税年。
 *
 * 在浏览模式下也在——换申报期是日常操作，不是编辑布局。
 * 改这里只动会话状态，不写布局文档。
 */
import { useEditor, usePageParams } from "./store";
import { currentTaxYearEnd, taxYearLabel, type ParamDef } from "./types";

/** 税年下拉的可选范围：当前税年往前六年，覆盖 IRD 的记录保存要求。 */
const TAX_YEAR_CHOICES = 7;

export function PageParamsBar({ pageId }: { pageId: string }) {
  const doc = useEditor((s) => s.doc);
  const setParam = useEditor((s) => s.setParam);
  const values = usePageParams(pageId);

  const page = doc.pages.find((p) => p.id === pageId);
  // hidden 的参数由块自己写，不给人看的控件。
  const shown = page?.params.filter((def) => def.control !== "hidden") ?? [];
  if (!page || shown.length === 0) return null;

  return (
    <div className="params-bar">
      {shown.map((def) => (
        <ParamControl
          key={def.key}
          def={def}
          value={values[def.key] ?? ""}
          onChange={(v) => setParam(pageId, def.key, v)}
        />
      ))}
    </div>
  );
}

function ParamControl({
  def,
  value,
  onChange,
}: {
  def: ParamDef;
  value: string;
  onChange: (value: string) => void;
}) {
  if (def.control === "tax-year") {
    const end = currentTaxYearEnd();
    const options = Array.from({ length: TAX_YEAR_CHOICES }, (_, i) => taxYearLabel(end - i));
    return (
      <label className="param">
        <span className="param-label">{def.label}</span>
        <select value={value} onChange={(e) => onChange(e.target.value)}>
          {/* 存档里可能是一个更早的税年，不在下拉范围内也要能显示出来。 */}
          {options.includes(value) ? null : <option value={value}>{value}</option>}
          {options.map((label) => (
            <option key={label} value={label}>
              {label}
            </option>
          ))}
        </select>
      </label>
    );
  }

  return (
    <label className="param">
      <span className="param-label">{def.label}</span>
      <input
        type={def.control === "date" ? "date" : "text"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      />
    </label>
  );
}
