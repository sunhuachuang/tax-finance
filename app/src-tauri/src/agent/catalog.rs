//! 块目录：agent 被允许摆放的东西，以及验证它有没有越界。
//!
//! 目录**不在 Rust 里写死**——前端启动时把组件注册表推过来。注册表是渲染器、
//! 属性面板和 agent 三者共用的同一份真相；在这里再抄一份迟早会和它漂移，
//! 而漂移的后果是 agent 生成出渲染器认不出的块。
//!
//! 这一层是 ARCHITECTURE.md 约束 1（布局是数据，不是代码）对 agent 的执行点：
//! 模型可以产出任意 JSON，但只有命中这张白名单的才落库。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockTypeInfo {
    pub r#type: String,
    pub name: String,
    pub hint: String,
    /// data / text / action / gate —— 决定它被允许做什么。
    pub kind: String,
    /// 锁定块不可删除（人工确认闸口）。
    #[serde(default)]
    pub locked: bool,
    /// 该块可配置的 props 键。
    #[serde(default)]
    pub prop_keys: Vec<String>,
    /// 该块可改写的文案键。
    #[serde(default)]
    pub copy_keys: Vec<String>,
    /// 默认绑定，给 agent 参考「这个块通常看什么数据」。
    #[serde(default)]
    pub default_binding: Option<Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BlockCatalog {
    pub blocks: Vec<BlockTypeInfo>,
    /// binding 允许指向的数据源名（前端 DATA_SOURCES 的键）。
    pub sources: Vec<String>,
}

impl BlockCatalog {
    pub fn get(&self, r#type: &str) -> Option<&BlockTypeInfo> {
        self.blocks.iter().find(|b| b.r#type == r#type)
    }

    /// 目录还没送过来时，一切校验都不该「通过」——没有白名单就等于没有约束。
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// 校验一份布局文档。返回所有问题，而不是遇到第一个就停——
    /// 一次性把话说完，agent 才能一轮改对。
    pub fn validate_doc(&self, doc: &Value) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();

        if self.is_empty() {
            return Err(vec![
                "块目录尚未就绪（前端还没注册组件表），此时无法校验布局，拒绝写入".to_string(),
            ]);
        }

        let Some(pages) = doc.get("pages").and_then(Value::as_array) else {
            return Err(vec!["布局文档缺少 pages 数组".to_string()]);
        };
        if pages.is_empty() {
            problems.push("布局至少要有一个页面".to_string());
        }

        // 锁定块（人工确认闸口）在整份文档里至少要留一个。
        let mut locked_seen: BTreeMap<&str, bool> = self
            .blocks
            .iter()
            .filter(|b| b.locked)
            .map(|b| (b.r#type.as_str(), false))
            .collect();

        let mut page_ids = Vec::new();
        for (pi, page) in pages.iter().enumerate() {
            match page.get("id").and_then(Value::as_str) {
                Some(id) if !id.is_empty() => {
                    if page_ids.contains(&id) {
                        problems.push(format!("页面 id 重复：{id}"));
                    }
                    page_ids.push(id);
                }
                _ => problems.push(format!("pages[{pi}] 缺少 id")),
            }

            let Some(blocks) = page.get("blocks").and_then(Value::as_array) else {
                problems.push(format!("pages[{pi}] 缺少 blocks 数组"));
                continue;
            };

            for (bi, block) in blocks.iter().enumerate() {
                let at = format!("pages[{pi}].blocks[{bi}]");

                // 与块型无关的结构检查先做。放在类型查找之后的话，一个不认识的
                // 块型会把 span、id 这些问题一起挡掉，agent 只能一轮改一个。
                if block.get("id").and_then(Value::as_str).is_none_or(str::is_empty) {
                    problems.push(format!("{at} 缺少 id"));
                }
                self.check_layout(&at, block, &mut problems);

                let Some(ty) = block.get("type").and_then(Value::as_str) else {
                    problems.push(format!("{at} 缺少 type"));
                    continue;
                };
                let Some(info) = self.get(ty) else {
                    problems.push(format!(
                        "{at} 用了未注册的块型 {ty}；可用的是：{}",
                        self.type_list()
                    ));
                    continue;
                };
                if let Some(seen) = locked_seen.get_mut(info.r#type.as_str()) {
                    *seen = true;
                }

                self.check_binding(&at, info, block, &mut problems);
            }
        }

        for (ty, seen) in locked_seen {
            if !seen {
                problems.push(format!(
                    "{ty} 是锁定块（人工确认闸口），不能从布局里移除"
                ));
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }

    fn check_layout(&self, at: &str, block: &Value, problems: &mut Vec<String>) {
        for (breakpoint, max) in [("desktop", 12), ("mobile", 4)] {
            match block
                .pointer(&format!("/layout/{breakpoint}/span"))
                .and_then(Value::as_i64)
            {
                Some(span) if (1..=max).contains(&span) => {}
                Some(span) => problems.push(format!(
                    "{at}.layout.{breakpoint}.span = {span} 越界，应在 1..={max}"
                )),
                None => problems.push(format!("{at}.layout.{breakpoint}.span 缺失")),
            }
        }
    }

    fn check_binding(
        &self,
        at: &str,
        info: &BlockTypeInfo,
        block: &Value,
        problems: &mut Vec<String>,
    ) {
        let Some(binding) = block.get("binding") else {
            // 数据块没有 binding 就没有数字可显示——这是空块，不是错误布局，
            // 但值得说一句，因为 agent 多半是漏了。
            if info.kind == "data" {
                problems.push(format!("{at} 是数据块但没有 binding，界面上会是空的"));
            }
            return;
        };
        if binding.is_null() {
            return;
        }

        match binding.get("source").and_then(Value::as_str) {
            Some(source) if self.sources.iter().any(|s| s == source) => {}
            Some(source) => problems.push(format!(
                "{at}.binding.source = {source} 不在允许的数据源里：{}",
                self.sources.join(", ")
            )),
            None => problems.push(format!("{at}.binding 缺少 source")),
        }

        if binding.get("path").and_then(Value::as_str).is_none() {
            problems.push(format!("{at}.binding 缺少 path（整个结果用空串）"));
        }
    }

    fn type_list(&self) -> String {
        self.blocks
            .iter()
            .map(|b| b.r#type.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn catalog() -> BlockCatalog {
        BlockCatalog {
            blocks: vec![
                BlockTypeInfo {
                    r#type: "stat".into(),
                    name: "数字".into(),
                    hint: "单个指标".into(),
                    kind: "data".into(),
                    locked: false,
                    prop_keys: vec!["suffix".into()],
                    copy_keys: vec!["title".into()],
                    default_binding: None,
                },
                BlockTypeInfo {
                    r#type: "review-gate".into(),
                    name: "确认闸口".into(),
                    hint: "批准 / 拒绝".into(),
                    kind: "gate".into(),
                    locked: true,
                    prop_keys: vec![],
                    copy_keys: vec![],
                    default_binding: None,
                },
            ],
            sources: vec!["overview".into(), "gst".into()],
        }
    }

    fn doc_with(blocks: Value) -> Value {
        json!({ "version": 1, "pages": [{ "id": "home", "title": "总览", "blocks": blocks }] })
    }

    fn stat() -> Value {
        json!({
            "id": "s1", "type": "stat",
            "layout": { "desktop": { "span": 3 }, "mobile": { "span": 2 } },
            "props": {},
            "binding": { "source": "overview", "path": "review_drafts", "agg": "count" }
        })
    }

    fn gate() -> Value {
        json!({
            "id": "g1", "type": "review-gate",
            "layout": { "desktop": { "span": 12 }, "mobile": { "span": 4 } },
            "props": {}
        })
    }

    #[test]
    fn a_well_formed_doc_passes() {
        assert!(catalog().validate_doc(&doc_with(json!([stat(), gate()]))).is_ok());
    }

    #[test]
    fn an_unregistered_block_type_is_rejected_and_the_options_are_listed() {
        let mut bad = stat();
        bad["type"] = json!("iframe");
        let problems = catalog()
            .validate_doc(&doc_with(json!([bad, gate()])))
            .unwrap_err();
        assert!(problems[0].contains("iframe"), "{problems:?}");
        // 报错要告诉 agent 能用什么，否则它只能瞎猜下一轮。
        assert!(problems[0].contains("stat"), "{problems:?}");
    }

    #[test]
    fn the_human_gate_cannot_be_removed() {
        let problems = catalog()
            .validate_doc(&doc_with(json!([stat()])))
            .unwrap_err();
        assert!(
            problems.iter().any(|p| p.contains("review-gate")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_binding_to_an_unknown_source_is_rejected() {
        let mut bad = stat();
        bad["binding"]["source"] = json!("shell");
        let problems = catalog()
            .validate_doc(&doc_with(json!([bad, gate()])))
            .unwrap_err();
        assert!(problems.iter().any(|p| p.contains("shell")), "{problems:?}");
    }

    #[test]
    fn an_out_of_range_span_is_rejected() {
        let mut bad = stat();
        bad["layout"]["desktop"]["span"] = json!(99);
        let problems = catalog()
            .validate_doc(&doc_with(json!([bad, gate()])))
            .unwrap_err();
        assert!(problems.iter().any(|p| p.contains("99")), "{problems:?}");
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let mut bad = stat();
        bad["type"] = json!("nope");
        bad["layout"]["mobile"]["span"] = json!(9);
        let problems = catalog().validate_doc(&doc_with(json!([bad]))).unwrap_err();
        // 未知块型 + span 越界 + 闸口缺失，一轮全报出来。
        assert!(problems.len() >= 3, "{problems:?}");
    }

    #[test]
    fn an_empty_catalog_refuses_everything() {
        let empty = BlockCatalog::default();
        assert!(empty.validate_doc(&doc_with(json!([stat()]))).is_err());
    }
}
