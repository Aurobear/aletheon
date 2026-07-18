//! MCP configuration uses the single canonical type owned by `cognit` and
//! re-exported by `corpus`; this helper preserves the existing call sites.

pub(crate) fn convert_mcp_servers(
    servers: &[cognit::config::McpServerConfig],
) -> Vec<corpus::tools::mcp::config::McpServerConfig> {
    servers.to_vec()
}
