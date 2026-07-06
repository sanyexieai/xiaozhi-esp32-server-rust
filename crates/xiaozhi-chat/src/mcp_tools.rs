//! MCP 工具列表过滤与 LLM 工具调用引导

use std::collections::HashSet;

use xiaozhi_llm::ToolInfo;

const FOOD_DELIVERY_KEYWORDS: &[&str] = &[
    "外卖", "点餐", "订餐", "吃什么", "点个", "叫外卖", "叫餐", "下单", "想吃", "午餐", "晚餐",
    "早餐", "夜宵", "附近吃", "推荐吃", "点一份", "帮我点",
];

const FOOD_TOOL_HINTS: &[&str] = &[
    "外卖", "点餐", "订餐", "餐厅", "菜单", "菜品", "waimai", "delivery", "restaurant", "food",
    "order", "meal", "dish",
];

/// 解析智能体配置的 MCP 服务名列表（逗号分隔）；空表示不限制、使用全部已启用服务。
pub fn parse_mcp_service_names(raw: &str) -> HashSet<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// 向对话注入 MCP 工具使用说明（有可用工具时）。
pub fn build_mcp_system_hint(tools: &[ToolInfo]) -> Option<String> {
    if tools.is_empty() {
        return None;
    }
    let has_food_tools = tools.iter().any(tool_matches_food_delivery);
    let mut hint = String::from(
        "你可以通过 function calling 调用已注册的工具完成查询与操作。\
         涉及实时数据、外部服务或设备能力时，应优先调用工具，不要凭空编造。",
    );
    if has_food_tools {
        hint.push_str(
            " 当用户询问外卖、点餐、吃什么、订餐、附近餐厅等需求时，\
             必须先调用外卖/点餐相关工具获取真实信息后再回复。",
        );
    }
    Some(hint)
}

/// 用户话术是否应走外卖/点餐类 MCP 工具。
pub fn user_text_prefers_food_delivery_tools(user_text: &str, tools: &[ToolInfo]) -> bool {
    let text = user_text.trim();
    if text.is_empty() || tools.is_empty() {
        return false;
    }
    if !FOOD_DELIVERY_KEYWORDS.iter().any(|kw| text.contains(kw)) {
        return false;
    }
    tools.iter().any(tool_matches_food_delivery) || tools.len() > 4
}

/// 模型直接文本回复时，追加一次工具调用提示。
pub fn food_delivery_tool_retry_hint() -> &'static str {
    "请调用可用的外卖/点餐相关工具查询真实数据后再回答用户，不要仅凭常识编造商家、菜品或价格。"
}

fn tool_matches_food_delivery(tool: &ToolInfo) -> bool {
    let combined = format!("{} {}", tool.name, tool.description).to_lowercase();
    FOOD_TOOL_HINTS
        .iter()
        .any(|hint| combined.contains(&hint.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mcp_service_names() {
        let set = parse_mcp_service_names("外卖, 麦当劳MCP Server,");
        assert_eq!(set.len(), 2);
        assert!(set.contains("外卖"));
        assert!(set.contains("麦当劳MCP Server"));
        assert!(parse_mcp_service_names("").is_empty());
    }

    #[test]
    fn detects_food_delivery_intent() {
        let tools = vec![ToolInfo {
            name: "get_menu".into(),
            description: "获取餐厅菜单".into(),
            parameters: serde_json::json!({}),
        }];
        assert!(user_text_prefers_food_delivery_tools("今天吃什么", &tools));
        assert!(!user_text_prefers_food_delivery_tools("今天天气怎么样", &tools));
    }
}
