/**
 * 块的注册入口。import 一次即完成注册（每个模块在自己文件里调 registerBlock）。
 *
 * 新增块型只需要：写一个文件，在文件末尾 registerBlock，然后在这里 import。
 * 渲染、属性面板、校验、序列化全部自动获得。
 */
import "./StatBlock";
import "./NoteBlock";
import "./RecordTableBlock";
import "./DocumentIntakeBlock";
import "./DocumentListBlock";
import "./DocumentDetailBlock";
import "./TaxReturnBlock";
import "./ReviewGateBlock";
