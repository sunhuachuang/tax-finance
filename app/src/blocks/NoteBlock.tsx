/**
 * 文本块。用户可以写任何文案，因此样式上必须明确表现为「注释」——
 * 它不能长得像一个数据块，否则就成了绕开 provenance 编数字的入口
 * （见 ARCHITECTURE.md 约束 2）。
 */
import { z } from "zod";

import { registerBlock, type BlockViewProps } from "../core/registry";

const propsSchema = z.object({
  level: z.enum(["heading", "note"]).default("note"),
});

function NoteBlock({ block, text }: BlockViewProps) {
  const props = propsSchema.parse(block.props);

  if (props.level === "heading") {
    return (
      <div className="block-body note">
        <h2 className="note-heading">{text("body")}</h2>
      </div>
    );
  }
  return (
    <div className="block-body note">
      <p className="note-text">
        <span className="note-marker">注</span>
        {text("body")}
      </p>
    </div>
  );
}

registerBlock({
  type: "note",
  name: "文本",
  hint: "自由文案。只是注释，不承载数据。",
  kind: "text",
  propsSchema,
  defaultProps: { level: "note" },
  defaultSpan: { desktop: 12, mobile: 4 },
  copy: [{ key: "body", label: "正文", fallback: "写点什么。" }],
  fields: [
    {
      key: "level",
      label: "层级",
      control: "select",
      options: [
        { value: "heading", label: "小标题" },
        { value: "note", label: "注释" },
      ],
    },
  ],
  Component: NoteBlock,
});
