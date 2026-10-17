//! The tools the MCP server offers (docs/mcp.md): their names, parameters
//! and JSON schemas are the contract AI clients are written against.

use super::{ToolError, ToolOutput};
use crate::AppState;
use rmcp::model::Tool;
use serde_json::Value;

/// Every tool, in the order clients see them.
pub fn catalog() -> Vec<Tool> {
    Vec::new()
}

/// Run `tool` with `arguments`, answering its output or the refusal the
/// client should read.
pub async fn dispatch(
    _state: &AppState,
    tool: &str,
    _arguments: Value,
) -> Result<ToolOutput, ToolError> {
    Err(ToolError(format!(
        "unknown tool '{tool}' — tools/list names the available ones"
    )))
}
