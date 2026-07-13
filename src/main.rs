use rust_mcp_sdk::error::SdkResult;
use rust_mcp_sdk::mcp_server::{server_runtime, ServerHandler};
use rust_mcp_sdk::schema::schema_utils::CallToolError;
use rust_mcp_sdk::schema::{
    CallToolRequestParams, CallToolResult, Implementation, InitializeResult, ListToolsResult,
    ProtocolVersion, ServerCapabilities, ServerCapabilitiesTools, Tool,
};
use rust_mcp_sdk::{McpServer, ToMcpServerHandler};
use rust_mcp_transport::{StdioTransport, TransportOptions};
use rust_mcp_axum::{create_axum_server, AxumServerOptions};
use clap::{Arg, Command};
use serde_json::json;
use std::sync::Arc;
use std::collections::HashSet;

mod config;
mod fetch;

#[cfg(feature = "embed-docs")]
use rust_embed::RustEmbed;

#[cfg(feature = "embed-docs")]
#[derive(RustEmbed)]
#[folder = "data/"]
struct DocsData;

/// The main MCP server handler for ChaosNexus Codex.
/// Maintains runtime state such as default pagination settings.
#[derive(Clone)]
struct Handler {
    default_offset: usize,
    default_limit: usize,
}

impl Handler {
    /// Creates a new `Handler` with the specified default pagination limits.
    fn new(default_offset: usize, default_limit: usize) -> Self {
        Self { default_offset, default_limit }
    }
    
    /// Retrieves a list of available documentation library names.
    /// Supports both embedded documentation (if compiled with `embed-docs`)
    /// and dynamically loaded libraries from the storage path.
    fn get_libs(&self) -> Vec<String> {
        let mut libs: HashSet<String> = HashSet::new();
        
        #[cfg(feature = "embed-docs")]
        {
            for file in DocsData::iter() {
                let Some(lib_name) = file.split('/').next() else { continue; };
                libs.insert(lib_name.to_string());
            }
        }

        #[cfg(not(feature = "embed-docs"))]
        {
            let config = config::CodexConfig::load();
            if let Ok(entries) = std::fs::read_dir(config.resolved_storage_path()) {
                for entry in entries.flatten() {
                    let Ok(ft) = entry.file_type() else { continue; };
                    if ft.is_dir() {
                        libs.insert(entry.file_name().to_string_lossy().to_string());
                    }
                }
            }
        }

        let mut libs_vec: Vec<String> = libs.into_iter().collect();
        libs_vec.sort();
        libs_vec
    }

    /// Retrieves a list of available markdown pages within a specific library.
    /// Filters out non-markdown files and returns just the file names.
    fn get_pages_in_lib(&self, library: &str) -> Vec<String> {
        let mut pages = Vec::new();
        
        #[cfg(feature = "embed-docs")]
        {
            let prefix = format!("{}/", library);
            for file in DocsData::iter() {
                if !file.starts_with(&prefix) { continue; }
                let path_str = file.as_ref();
                if !path_str.ends_with(".md") && !path_str.ends_with(".mdx") { continue; }
                
                let Some(name) = path_str.split('/').next_back() else { continue; };
                pages.push(name.to_string());
            }
        }

        #[cfg(not(feature = "embed-docs"))]
        {
            let config = config::CodexConfig::load();
            let lib_dir = config.resolved_storage_path().join(library);
            if let Ok(entries) = std::fs::read_dir(lib_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if ext == "md" || ext == "mdx" {
                        pages.push(entry.file_name().to_string_lossy().to_string());
                    }
                }
            }
        }

        pages
    }
}

/// Helper function to safely extract the arguments object from a tool call request.
fn get_args<'a>(
    params: &'a CallToolRequestParams,
    tool_name: &str,
) -> Result<&'a serde_json::Map<String, serde_json::Value>, CallToolError> {
    params.arguments.as_ref().ok_or_else(|| {
        CallToolError::invalid_arguments(tool_name, Some("Missing arguments object".to_string()))
    })
}

/// Helper function to safely extract a required string argument from a tool call request.
fn get_string_arg<'a>(
    args: &'a serde_json::Map<String, serde_json::Value>,
    tool_name: &str,
    arg_name: &str,
) -> Result<&'a str, CallToolError> {
    args.get(arg_name)
        .ok_or_else(|| {
            CallToolError::invalid_arguments(
                tool_name,
                Some(format!("Missing '{}' argument", arg_name)),
            )
        })?
        .as_str()
        .ok_or_else(|| {
            CallToolError::invalid_arguments(
                tool_name,
                Some(format!("'{}' must be a string", arg_name)),
            )
        })
}


#[async_trait::async_trait]
impl ServerHandler for Handler {
    async fn handle_call_tool_request(
        &self,
        params: CallToolRequestParams,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<CallToolResult, CallToolError> {
        let name = params.name.as_str();
        match name {
            "list_pages" => {
                tracing::info!("[ChaosNexus Codex] 🛠️  Called 'list_pages' with arguments: {:?}", params.arguments);
                let args = get_args(&params, name)?;
                let library = get_string_arg(args, name, "library")?;
                
                let pages = self.get_pages_in_lib(library);
                tracing::info!("[ChaosNexus Codex] 📄  'list_pages' found {} pages in library '{}'", pages.len(), library);
                
                if pages.is_empty() {
                    return Ok(CallToolResult::text_content(vec![
                        rust_mcp_sdk::schema::TextContent::from(format!(
                            "Error: Library '{}' not found or empty",
                            library
                        )),
                    ]));
                }
                
                Ok(CallToolResult::text_content(vec![
                    rust_mcp_sdk::schema::TextContent::from(
                        serde_json::to_string(&pages).unwrap(),
                    ),
                ]))
            }
            "search_docs" => {
                tracing::info!("[ChaosNexus Codex] 🛠️  Called 'search_docs' with arguments: {:?}", params.arguments);
                let args = get_args(&params, name)?;
                let library = get_string_arg(args, name, "library")?;
                let query_str = get_string_arg(args, name, "query")?;
                let query = query_str.to_lowercase();

                let all_pages = self.get_pages_in_lib(library);
                if all_pages.is_empty() {
                    tracing::error!("[ChaosNexus Codex] ❌  'search_docs' failed: Library '{}' not found", library);
                    return Ok(CallToolResult::text_content(vec![
                        rust_mcp_sdk::schema::TextContent::from(format!(
                            "Error: Library '{}' not found or empty",
                            library
                        )),
                    ]));
                }

                let matches: Vec<String> = all_pages
                    .into_iter()
                    .filter(|name| name.to_lowercase().contains(&query))
                    .collect();

                tracing::info!("[ChaosNexus Codex] 🔍  'search_docs' found {} matches for query '{}'", matches.len(), query);
                Ok(CallToolResult::text_content(vec![
                    rust_mcp_sdk::schema::TextContent::from(
                        serde_json::to_string(&matches).unwrap(),
                    ),
                ]))
            }
            "read_doc_page" => {
                tracing::info!("[ChaosNexus Codex] 🛠️  Called 'read_doc_page' with arguments: {:?}", params.arguments);
                let args = get_args(&params, name)?;
                let library = get_string_arg(args, name, "library")?;
                let page = get_string_arg(args, name, "page")?;

                let offset: usize = args.get("offset")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize)
                    .unwrap_or(self.default_offset);
                
                let limit: usize = args.get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize)
                    .unwrap_or(self.default_limit);

                let path = format!("{}/{}", library, page);

                let mut content = String::new();
                
                #[cfg(feature = "embed-docs")]
                {
                    if let Some(file) = DocsData::get(&path) {
                        if let Ok(utf8_content) = std::str::from_utf8(&file.data) {
                            content = utf8_content.to_string();
                        } else {
                            tracing::error!("[ChaosNexus Codex] ❌  'read_doc_page' failed: Invalid UTF-8 in {}", path);
                            return Ok(CallToolResult::text_content(vec![
                                rust_mcp_sdk::schema::TextContent::from("Error reading file: Invalid UTF-8".to_string()),
                            ]));
                        }
                    }
                }

                #[cfg(not(feature = "embed-docs"))]
                {
                    let config = config::CodexConfig::load();
                    let file_path = config.resolved_storage_path().join(library).join(page);
                    if let Ok(fs_content) = std::fs::read_to_string(&file_path) {
                        content = fs_content;
                    }
                }

                if content.is_empty() {
                    tracing::error!("[ChaosNexus Codex] ❌  'read_doc_page' failed: Page '{}' not found", path);
                    return Ok(CallToolResult::text_content(vec![
                        rust_mcp_sdk::schema::TextContent::from(format!(
                            "Error: Page '{}' in library '{}' not found",
                            page, library
                        )),
                    ]));
                }

                let original_len = content.chars().count();
                let start_idx = content.char_indices().nth(offset).map(|(i, _)| i).unwrap_or(content.len());
                let end_idx = content[start_idx..].char_indices().nth(limit).map(|(i, _)| start_idx + i).unwrap_or(content.len());
                
                let mut text = content[start_idx..end_idx].to_string();

                if end_idx < content.len() {
                    tracing::warn!("[ChaosNexus Codex] 📖  'read_doc_page' returning chunk for '{}' (offset {}, length {} -> {} chars total)", path, offset, text.len(), original_len);
                    text.push_str(&format!("\n\n...[TRUNCATED: Document has more content. Use offset={} to read next chunk.]", offset + limit));
                } else {
                    tracing::info!("[ChaosNexus Codex] 📖  'read_doc_page' returning end chunk for '{}' (offset {}, length {} -> {} chars total)", path, offset, text.len(), original_len);
                }

                Ok(CallToolResult::text_content(vec![
                    rust_mcp_sdk::schema::TextContent::from(text),
                ]))
            }
            "add_docs" => {
                #[cfg(feature = "embed-docs")]
                return Err(CallToolError::invalid_arguments(name, Some("add_docs is not supported when running with embed-docs feature.".to_string())));

                #[cfg(not(feature = "embed-docs"))]
                {
                    let Some(args) = params.arguments.as_ref() else {
                        return Err(CallToolError::invalid_arguments(name, Some("Missing arguments".to_string())));
                    };
                    let repo_url = args.get("repo_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let sub_dir = args.get("sub_dir").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let dst_folder = args.get("dst_folder").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let use_sparse = args.get("use_sparse").and_then(|v| v.as_bool()).unwrap_or(false);

                    if repo_url.is_empty() || dst_folder.is_empty() {
                        return Err(CallToolError::invalid_arguments(name, Some("repo_url and dst_folder are required".to_string())));
                    }

                    let mut config = crate::config::CodexConfig::load();
                    config.libraries.retain(|l| l.dst_folder != dst_folder);
                    config.libraries.push(crate::config::LibraryConfig {
                        repo_url, sub_dir, dst_folder, use_sparse
                    });
                    
                    if let Err(e) = config.save() {
                        return Ok(CallToolResult::text_content(vec![rust_mcp_sdk::schema::TextContent::from(format!("Failed to save config: {}", e))]));
                    }

                    tokio::spawn(async move {
                        if let Err(e) = crate::fetch::fetch_all_docs().await {
                            tracing::error!("[ChaosNexus Codex] Fetch failed during add_docs: {}", e);
                        }
                    });

                    Ok(CallToolResult::text_content(vec![rust_mcp_sdk::schema::TextContent::from("Library added to configuration. Fetching in the background...".to_string())]))
                }
            }
            "remove_docs" => {
                #[cfg(feature = "embed-docs")]
                return Err(CallToolError::invalid_arguments(name, Some("remove_docs is not supported when running with embed-docs feature.".to_string())));

                #[cfg(not(feature = "embed-docs"))]
                {
                    let Some(args) = params.arguments.as_ref() else {
                        return Err(CallToolError::invalid_arguments(name, Some("Missing arguments".to_string())));
                    };
                    let dst_folder = args.get("dst_folder").and_then(|v| v.as_str()).unwrap_or("").to_string();

                    let mut config = crate::config::CodexConfig::load();
                    let orig_len = config.libraries.len();
                    config.libraries.retain(|l| l.dst_folder != dst_folder);
                    
                    if config.libraries.len() == orig_len {
                        return Ok(CallToolResult::text_content(vec![rust_mcp_sdk::schema::TextContent::from(format!("Library '{}' not found in configuration.", dst_folder))]));
                    }

                    if let Err(e) = config.save() {
                        return Ok(CallToolResult::text_content(vec![rust_mcp_sdk::schema::TextContent::from(format!("Failed to save config: {}", e))]));
                    }

                    let dst_dir = config.resolved_storage_path().join(&dst_folder);
                    let _ = std::fs::remove_dir_all(&dst_dir);

                    Ok(CallToolResult::text_content(vec![rust_mcp_sdk::schema::TextContent::from(format!("Library '{}' removed successfully.", dst_folder))]))
                }
            }
            "chaosdocs_get_status" => {
                let config = crate::config::CodexConfig::load();
                let config_json = serde_json::to_value(&config)
                    .map_err(|e| CallToolError::from_message(format!("Failed to serialize config: {}", e)))?;
                
                Ok(CallToolResult::text_content(vec![rust_mcp_sdk::schema::TextContent::from(format!(
                    "{{\n  \"storage_path\": \"{}\",\n  \"config\": {}\n}}",
                    config.resolved_storage_path().display(),
                    serde_json::to_string_pretty(&config_json).unwrap_or_default()
                ))]))
            }
            _ => {
                tracing::error!("[ChaosNexus Codex] ❌  Unknown tool called: {}", name);
                Err(CallToolError::unknown_tool(name.to_string()))
            }
        }
    }

    async fn handle_list_tools_request(
        &self,
        _params: Option<rust_mcp_sdk::schema::PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<ListToolsResult, rust_mcp_sdk::schema::RpcError> {
        let libs = self.get_libs();

        let mut tools = vec![
            Tool {
                name: "list_pages".to_string(),
                description: Some(
                    "List all available documentation pages for a specific library.".to_string(),
                ),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "library": { "type": "string", "enum": libs }
                    },
                    "required": ["library"]
                }))
                .unwrap(),
                meta: None,
                output_schema: None,
                title: None,
                annotations: None,
                execution: None,
                icons: Vec::new(),
            },
            Tool {
                name: "search_docs".to_string(),
                description: Some("Search for documentation pages within a specific library by matching the query against filenames.".to_string()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "library": { "type": "string", "enum": libs },
                        "query": { "type": "string" }
                    },
                    "required": ["library", "query"]
                })).unwrap(),
                meta: None, output_schema: None, title: None, annotations: None, execution: None, icons: Vec::new(),
            },
            Tool {
                name: "read_doc_page".to_string(),
                description: Some("Read the contents of a specific documentation page. Provide exact library and page name. Supports offset and limit for pagination.".to_string()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "library": { "type": "string", "enum": libs },
                        "page": { "type": "string" },
                        "offset": { "type": "integer", "description": "Character offset to start reading from. Default is 0." },
                        "limit": { "type": "integer", "description": "Maximum number of characters to read. Default is 16000." }
                    },
                    "required": ["library", "page"]
                })).unwrap(),
                meta: None, output_schema: None, title: None, annotations: None, execution: None, icons: Vec::new(),
            },
        ];

        #[cfg(not(feature = "embed-docs"))]
        {
            tools.push(Tool {
                name: "add_docs".to_string(),
                description: Some("Add a new documentation library to the configuration and fetch it immediately (only works if not compiled with embed-docs).".to_string()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "repo_url": { "type": "string", "description": "Git repository URL" },
                        "sub_dir": { "type": "string", "description": "Subdirectory to extract from the repo (e.g., 'src' or 'docs')" },
                        "dst_folder": { "type": "string", "description": "Destination folder name for the library (e.g., 'rhai')" },
                        "use_sparse": { "type": "boolean", "description": "Whether to use sparse checkout to only download the sub_dir" }
                    },
                    "required": ["repo_url", "sub_dir", "dst_folder", "use_sparse"]
                })).unwrap(),
                meta: None, output_schema: None, title: None, annotations: None, execution: None, icons: Vec::new(),
            });
            tools.push(Tool {
                name: "remove_docs".to_string(),
                description: Some("Remove a documentation library from the configuration (only works if not compiled with embed-docs).".to_string()),
                input_schema: serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "dst_folder": { "type": "string", "description": "Destination folder name of the library to remove" }
                    },
                    "required": ["dst_folder"]
                })).unwrap(),
                meta: None, output_schema: None, title: None, annotations: None, execution: None, icons: Vec::new(),
            });
        }

        tools.push(Tool {
            name: "chaosdocs_get_status".to_string(),
            description: Some("Returns the active read-only configuration settings and the absolute path to the documentation storage directory. Use this to orient the agent to the environment.".to_string()),
            input_schema: serde_json::from_value(json!({
                "type": "object",
                "properties": {}
            })).unwrap(),
            meta: None, output_schema: None, title: None, annotations: None, execution: None, icons: Vec::new(),
        });

        Ok(ListToolsResult {
            tools,
            next_cursor: None,
            meta: None,
        })
    }

    async fn handle_list_resources_request(
        &self,
        _params: Option<rust_mcp_sdk::schema::PaginatedRequestParams>,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<rust_mcp_sdk::schema::ListResourcesResult, rust_mcp_sdk::schema::RpcError> {
        let mut resources: Vec<rust_mcp_sdk::schema::Resource> = Vec::new();

        #[cfg(feature = "embed-docs")]
        {
            for file in DocsData::iter() {
                let path_str = file.as_ref();
                if !path_str.ends_with(".md") && !path_str.ends_with(".mdx") { continue; }
                
                let mut parts = path_str.split('/');
                let Some(lib_name) = parts.next() else { continue; };
                let Some(page_name) = parts.next_back() else { continue; };

                resources.push(rust_mcp_sdk::schema::Resource {
                    uri: format!("docs://{}/{}", lib_name, page_name),
                    name: page_name.to_string(),
                    description: Some(format!("Documentation page in {}", lib_name)),
                    mime_type: Some("text/markdown".to_string()),
                    size: None,
                    annotations: None,
                    meta: None,
                    icons: vec![],
                    title: None,
                });
            }
        }

        #[cfg(not(feature = "embed-docs"))]
        {
            let config = config::CodexConfig::load();
            if let Ok(entries) = std::fs::read_dir(config.resolved_storage_path()) {
                for entry in entries.flatten() {
                    let Ok(ft) = entry.file_type() else { continue; };
                    if !ft.is_dir() { continue; }
                    let lib_name = entry.file_name().to_string_lossy().to_string();
                    let lib_dir = entry.path();
                    let Ok(files) = std::fs::read_dir(lib_dir) else { continue; };
                    for file in files.flatten() {
                        let path = file.path();
                        if !path.is_file() { continue; }
                        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                        if ext == "md" || ext == "mdx" {
                            let page_name = file.file_name().to_string_lossy().to_string();
                            resources.push(rust_mcp_sdk::schema::Resource {
                                uri: format!("docs://{}/{}", lib_name, page_name),
                                name: page_name.clone(),
                                description: Some(format!("Documentation page in {}", lib_name)),
                                mime_type: Some("text/markdown".to_string()),
                                size: None,
                                annotations: None,
                                meta: None,
                                icons: vec![],
                                title: None,
                            });
                        }
                    }
                }
            }
        }

        Ok(rust_mcp_sdk::schema::ListResourcesResult {
            resources,
            next_cursor: None,
            meta: None,
        })
    }

    async fn handle_read_resource_request(
        &self,
        params: rust_mcp_sdk::schema::ReadResourceRequestParams,
        _runtime: Arc<dyn McpServer>,
    ) -> Result<rust_mcp_sdk::schema::ReadResourceResult, rust_mcp_sdk::schema::RpcError> {
        let uri = params.uri;
        let Some(path_str) = uri.strip_prefix("docs://") else {
            return Err(rust_mcp_sdk::schema::RpcError::new(
                rust_mcp_sdk::schema::RpcErrorCodes::INVALID_REQUEST,
                "Invalid URI prefix".to_string(),
                None,
            ));
        };

        let parts: Vec<&str> = path_str.split('/').collect();
        if parts.len() != 2 {
            return Err(rust_mcp_sdk::schema::RpcError::new(
                rust_mcp_sdk::schema::RpcErrorCodes::INVALID_REQUEST,
                "Resource not found".to_string(),
                None,
            ));
        }

        let library = parts[0];
        let page = parts[1];
        let path = format!("{}/{}", library, page);
        #[allow(unused_variables)]
        let _p = &path;

        let mut content = String::new();

        #[cfg(feature = "embed-docs")]
        {
            if let Some(file) = DocsData::get(&path) {
                if let Ok(utf8) = std::str::from_utf8(&file.data) {
                    content = utf8.to_string();
                } else {
                    return Err(rust_mcp_sdk::schema::RpcError::new(
                        rust_mcp_sdk::schema::RpcErrorCodes::INTERNAL_ERROR,
                        "Invalid UTF-8".to_string(),
                        None,
                    ));
                }
            }
        }

        #[cfg(not(feature = "embed-docs"))]
        {
            let config = config::CodexConfig::load();
            let file_path = config.resolved_storage_path().join(library).join(page);
            if let Ok(fs_content) = std::fs::read_to_string(&file_path) {
                content = fs_content;
            }
        }

        if content.is_empty() {
            return Err(rust_mcp_sdk::schema::RpcError::new(
                rust_mcp_sdk::schema::RpcErrorCodes::INVALID_REQUEST,
                "Resource not found".to_string(),
                None,
            ));
        }

        Ok(rust_mcp_sdk::schema::ReadResourceResult {
            contents: vec![
                rust_mcp_sdk::schema::ReadResourceContent::TextResourceContents(
                    rust_mcp_sdk::schema::TextResourceContents {
                        uri,
                        mime_type: Some("text/markdown".to_string()),
                        text: content.to_string(),
                        meta: None,
                    },
                ),
            ],
            meta: None,
        })
    }
}

/// The main entry point for the ChaosNexus Codex MCP Server.
/// Handles CLI argument parsing, configuration loading, and server initialization.
/// Can run in either stdio mode (default) or HTTP SSE mode (if port is specified).
#[tokio::main]
async fn main() -> SdkResult<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new(
                    "info,chaosnexus_codex=debug,rust_mcp_axum=debug,rust_mcp_sdk=debug",
                )
            }),
        )
        .init();

    let server_details = InitializeResult {
        server_info: Implementation {
            name: "chaosnexus-codex".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            title: Some("ChaosNexus Codex MCP Server".to_string()),
            description: Some(
                "Provides hallucination-free access to documentation for ChaosNexus Anvil and ChaosNexus Forge libraries.".to_string(),
            ),
            icons: vec![],
            website_url: None,
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools {
                list_changed: Some(false),
            }),
            resources: Some(rust_mcp_sdk::schema::ServerCapabilitiesResources {
                subscribe: Some(false),
                list_changed: Some(false),
            }),
            ..Default::default()
        },
        meta: None,
        protocol_version: ProtocolVersion::V2025_11_25.into(),
        instructions: Some(
            "Use search_docs to find relevant files, and read_doc_page to extract markdown contents.".to_string(),
        ),
    };

    let matches = Command::new("chaosnexus-codex")
        .subcommand(Command::new("fetch").about("Fetches and syncs documentation according to chaosnexus-codex.toml"))
        .arg(
            Arg::new("port")
                .long("port")
                .value_parser(clap::value_parser!(u16))
                .help("Run as an SSE HTTP server on the given port"),
        )
        .arg(
            Arg::new("default-offset")
                .long("default-offset")
                .value_parser(clap::value_parser!(usize))
                .default_value("0")
                .help("Default character offset for reading documentation pages"),
        )
        .arg(
            Arg::new("default-limit")
                .long("default-limit")
                .value_parser(clap::value_parser!(usize))
                .default_value("16000")
                .help("Default maximum character limit for reading documentation pages"),
        )
        .get_matches();

    if let Some(_fetch_matches) = matches.subcommand_matches("fetch") {
        if let Err(e) = fetch::fetch_all_docs().await {
            tracing::error!("Fetch failed: {}", e);
            std::process::exit(1);
        }
        return Ok(());
    }

    let config = config::CodexConfig::load();

    let default_offset = *matches.get_one::<usize>("default-offset").unwrap_or(&config.default_offset.unwrap_or(0));
    let default_limit = *matches.get_one::<usize>("default-limit").unwrap_or(&config.default_limit.unwrap_or(16000));

    if let Some(start_port) = matches.get_one::<u16>("port").copied().or(config.port) {
        let mut port = start_port;
        let max_port = start_port + 100;
        let mut success = false;

        while port <= max_port {
            if std::net::TcpListener::bind(("0.0.0.0", port)).is_ok() {
                tracing::info!("[ChaosNexus Codex] 🚀 Starting SSE server on port {}", port);
                let handler = Handler::new(default_offset, default_limit).to_mcp_server_handler();
                let server = create_axum_server(
                    server_details.clone(),
                    handler,
                    AxumServerOptions {
                        host: "0.0.0.0".to_string(),
                        port,
                        ..Default::default()
                    },
                );
                server.start().await?;
                success = true;
                break;
            }
            tracing::warn!("[ChaosNexus Codex] ⚠️ Port {} is in use, trying next...", port);
            port += 1;
        }

        if !success {
            tracing::error!("[ChaosNexus Codex] ❌ Failed to find an open port in range {}..{}", start_port, max_port);
            std::process::exit(1);
        }
    } else {
        tracing::info!("[ChaosNexus Codex] no port defined, defaulting to stdio");
        let transport = StdioTransport::new(TransportOptions::default())?;
        let handler = Handler::new(default_offset, default_limit).to_mcp_server_handler();

        let server = server_runtime::create_server(rust_mcp_sdk::mcp_server::McpServerOptions {
            server_details,
            transport,
            handler,
            task_store: None,
            client_task_store: None,
            message_observer: None,
        });

        server.start().await?;
    }
    
    Ok(())
}
