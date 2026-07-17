//! Conversion from cognit (TOML-facing) MCP server configs to the
//! canonical corpus runtime type.

/// Convert a single cognit-flavored MCP server config into its corpus
/// equivalent.  All MCP servers defined in user-facing config are treated as
/// `LocalTrusted` and enabled by default.
pub(crate) fn convert_mcp_server(
    server: &cognit::config::McpServerConfig,
) -> corpus::tools::mcp::config::McpServerConfig {
    use corpus::tools::mcp::config::{McpServerConfig, McpTransportConfig, McpTrustLevel};

    McpServerConfig {
        name: server.name.clone(),
        transport: match server.transport.as_str() {
            "stdio" => McpTransportConfig::Stdio {
                command: server.command.clone().unwrap_or_default(),
                args: Vec::new(),
            },
            "http" => McpTransportConfig::StreamableHttp {
                url: server.url.clone().unwrap_or_default(),
            },
            "sse" => McpTransportConfig::Sse {
                url: server.url.clone().unwrap_or_default(),
            },
            _ => McpTransportConfig::Stdio {
                command: server.command.clone().unwrap_or_default(),
                args: Vec::new(),
            },
        },
        trust: McpTrustLevel::LocalTrusted,
        enabled: true,
        bearer_token_env: server.bearer_token_env.clone(),
    }
}

/// Convenience: convert a slice of cognit configs into a `Vec` of corpus
/// configs.
pub(crate) fn convert_mcp_servers(
    servers: &[cognit::config::McpServerConfig],
) -> Vec<corpus::tools::mcp::config::McpServerConfig> {
    servers.iter().map(convert_mcp_server).collect()
}
