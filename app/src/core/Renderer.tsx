/**
 * 渲染器：把布局文档变成界面。
 *
 * 它对具体的块一无所知——只认注册表。命不中注册表的 `type` 渲染成占位错误块，
 * 绝不尝试执行任何来自文档的东西（ARCHITECTURE.md 约束 1）。
 */
import { useMemo } from "react";
import {
  DndContext,
  KeyboardSensor,
  PointerSensor,
  closestCenter,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import { restrictToParentElement } from "@dnd-kit/modifiers";
import {
  SortableContext,
  rectSortingStrategy,
  sortableKeyboardCoordinates,
  useSortable,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";

import { useData, useSource } from "./data";
import { getBlockDef } from "./registry";
import { useEditor, usePageParams } from "./store";
import {
  DESKTOP_COLUMNS,
  MOBILE_COLUMNS,
  resolveBinding,
  resolveBindingParams,
  type Block,
  type Breakpoint,
} from "./types";

export function PageRenderer() {
  const doc = useEditor((s) => s.doc);
  const pageId = useEditor((s) => s.pageId);
  const mode = useEditor((s) => s.mode);
  const breakpoint = useEditor((s) => s.breakpoint);
  const moveBlock = useEditor((s) => s.moveBlock);

  const page = doc.pages.find((p) => p.id === pageId) ?? doc.pages[0];
  const ids = useMemo(() => page.blocks.map((b) => b.id), [page.blocks]);

  const sensors = useSensors(
    // 有一点位移才算拖拽，否则编辑模式下点击选中会被吞掉。
    useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );

  const columns = breakpoint === "desktop" ? DESKTOP_COLUMNS : MOBILE_COLUMNS;
  const grid = (
    <div className="grid" style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}>
      {page.blocks.map((block) => (
        <BlockFrame key={block.id} block={block} breakpoint={breakpoint} sortable={mode === "edit"} />
      ))}
    </div>
  );

  if (mode !== "edit") return grid;

  function onDragEnd(event: DragEndEvent) {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    moveBlock(ids.indexOf(String(active.id)), ids.indexOf(String(over.id)));
  }

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={closestCenter}
      modifiers={[restrictToParentElement]}
      onDragEnd={onDragEnd}
    >
      <SortableContext items={ids} strategy={rectSortingStrategy}>
        {grid}
      </SortableContext>
    </DndContext>
  );
}

function BlockFrame({
  block,
  breakpoint,
  sortable,
}: {
  block: Block;
  breakpoint: Breakpoint;
  sortable: boolean;
}) {
  const selectedId = useEditor((s) => s.selectedId);
  const select = useEditor((s) => s.select);

  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({
    id: block.id,
    disabled: !sortable,
  });

  const def = getBlockDef(block.type);
  const selected = selectedId === block.id;

  const style: React.CSSProperties = {
    gridColumn: `span ${block.layout[breakpoint].span}`,
    transform: CSS.Translate.toString(transform),
    transition,
    opacity: isDragging ? 0.4 : 1,
  };

  return (
    <div
      ref={setNodeRef}
      style={style}
      className={[
        "block-frame",
        selected ? "selected" : "",
        sortable ? "editable" : "",
        def ? `kind-${def.kind}` : "kind-unknown",
      ]
        .filter(Boolean)
        .join(" ")}
      onClick={sortable ? () => select(block.id) : undefined}
    >
      {sortable ? (
        <div className="block-handle" {...attributes} {...listeners} title="拖动重排">
          <span className="handle-dots">⠿</span>
          <span className="handle-name">{def?.name ?? block.type}</span>
          {def?.locked ? <span className="handle-lock">锁定</span> : null}
        </div>
      ) : null}
      <BlockHost block={block} editing={sortable} />
    </div>
  );
}

function BlockHost({ block, editing }: { block: Block; editing: boolean }) {
  const data = useData();
  const pageId = useEditor((s) => s.pageId);
  const pageParams = usePageParams(pageId);
  const setStoreParam = useEditor((s) => s.setParam);

  // hooks 必须无条件调用，所以取数在「块型是否存在」的判断之前。
  const params = block.binding ? resolveBindingParams(block.binding, pageParams) : {};
  const state = useSource(block.binding?.source ?? null, params);

  const def = getBlockDef(block.type);
  if (!def) {
    // 未注册的块型。不猜、不渲染任何来自文档的内容，只报出问题。
    return (
      <div className="block-body block-unknown">
        <div className="block-title">未知块型</div>
        <div className="block-error">
          布局里引用了未注册的块型 <code>{block.type}</code>。可能是旧版本存档，或文档被手改过。
        </div>
      </div>
    );
  }

  const value = block.binding ? resolveBinding(state.data, block.binding) : undefined;

  const text = (key: string) =>
    block.copy?.[key] ?? def.copy?.find((slot) => slot.key === key)?.fallback ?? "";

  const refresh = () => {
    if (block.binding) data.refresh(block.binding.source, params);
  };

  return (
    <def.Component
      block={block}
      value={value}
      text={text}
      loading={state.loading}
      error={state.error}
      editing={editing}
      refresh={refresh}
      params={pageParams}
      setParam={(key, value) => setStoreParam(pageId, key, value)}
    />
  );
}
