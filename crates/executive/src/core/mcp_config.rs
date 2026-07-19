//! MCP configuration uses the single canonical type owned by Corpus.

pub(crate) fn convert_mcp_servers(
    servers: &[corpus::tools::mcp::config::McpServerConfig],
) -> Vec<corpus::tools::mcp::config::McpServerConfig> {
    servers.to_vec()
}
