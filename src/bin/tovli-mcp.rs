//! Local MCP server for the durable ingestion application service.
use std::path::{Path, PathBuf};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::Arc;

use base64::{engine::general_purpose::STANDARD, Engine};
use clap::Parser;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, ErrorData, ListToolsResult, PaginatedRequestParam,
    ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, ServiceExt};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};
use tovli::image_memory::links;
use tovli::ingestion::durable;
use tovli::ingestion::infra::mock_embedder::MockEmbedder;
use tovli::ingestion::ports::Embedder;

const MAX_FRAME: usize = 16 * 1024 * 1024;
const MAX_TEXT_REQUEST: usize = 1024 * 1024;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    store_dir: PathBuf,
    #[arg(long)]
    mock: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryInput {
    source_key: String,
}

enum Work {
    Add(durable::Input),
    Query(String),
    AddImage(durable::ImageInput, Vec<u8>),
    GetImage(String),
    Link(links::LinkInput),
    ListLinks(links::ListInput),
}

struct Job {
    work: Work,
    reply: oneshot::Sender<Result<Value, String>>,
    _image_permit: Option<OwnedSemaphorePermit>,
}

#[derive(Clone)]
struct Server {
    sender: SyncSender<Job>,
    image_budget: Arc<Semaphore>,
}

fn make_embedder(mock: bool) -> anyhow::Result<Box<dyn Embedder>> {
    if mock {
        return Ok(Box::new(MockEmbedder::new(384)));
    }
    #[cfg(feature = "onnx")]
    {
        Ok(Box::new(
            tovli::ingestion::infra::onnx_embedder::OnnxEmbedder::open_minilm()?,
        ))
    }
    #[cfg(not(feature = "onnx"))]
    {
        anyhow::bail!("ONNX is disabled; run with --mock or enable the onnx feature")
    }
}

fn worker(root: &Path, mock: bool, receiver: std::sync::mpsc::Receiver<Job>) {
    let mut embedder: Option<Box<dyn Embedder>> = None;
    for job in receiver {
        if job.reply.is_closed() {
            continue;
        }
        let result = match job.work {
            Work::Add(input) => {
                let service = match embedder {
                    Some(ref service) => Ok(service.as_ref()),
                    None => match make_embedder(mock) {
                        Ok(value) => {
                            embedder = Some(value);
                            Ok(embedder.as_ref().expect("embedder initialized").as_ref())
                        }
                        Err(error) => Err(error),
                    },
                };
                service.and_then(|service| durable::add(root, service, input))
                    .and_then(|receipt| Ok(serde_json::to_value(receipt)?))
            }
            Work::Query(key) => durable::query(root, &key)
                .map(|result| json!({"results": result.into_iter().collect::<Vec<_>>() })),
            Work::AddImage(input, bytes) => {
                if embedder.is_none() {
                    embedder = make_embedder(mock).ok();
                }
                match embedder.as_ref() {
                    Some(service) => durable::add_image(root, service.as_ref(), input, bytes),
                    None => Err(anyhow::anyhow!("embedding_unavailable")),
                }
            }
            Work::GetImage(id) => durable::get_image(root, &id).map(|(asset, bytes)| {
                json!({"image_base64": STANDARD.encode(bytes), "mime_type": asset.mime_type, "asset_id": asset.asset_id})
            }),
            Work::Link(input) => links::create_for_store(root, &input).map(|(link, created)| {
                let mut value = serde_json::to_value(link).expect("serializable link");
                value["status"] = json!(if created { "created" } else { "unchanged" });
                value
            }),
            Work::ListLinks(input) => links::list_page_for_store(root, input)
                .and_then(|page| Ok(serde_json::to_value(page)?)),
        };
        let _ = job.reply.send(result.map_err(|error| format!("{error:#}")));
    }
}

fn schema(value: Value) -> Map<String, Value> {
    value.as_object().expect("schema is an object").clone()
}

fn tool(name: &'static str, description: &'static str, input: Value, read_only: bool) -> Tool {
    let mut definition = Tool::new(name, description, schema(input));
    definition.annotations = Some(
        ToolAnnotations::new()
            .read_only(read_only)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
    );
    if name != "tovli_get_image" {
        let success = match name {
            "tovli_add_document" | "tovli_add_image" => json!({
                "type":"object",
                "required":["status","document_id","source_uri","content_hash","chunk_count",
                            "embedding_model","embedding_dimension","indexed_at"],
                "properties":{"status":{"enum":["created","unchanged"]},
                    "document_id":{"type":"string"},"source_uri":{"type":"string"},
                    "content_hash":{"type":"string"},"chunk_count":{"type":"integer"},
                    "embedding_model":{"type":"string"},"embedding_dimension":{"type":"integer"},
                    "indexed_at":{"type":"string"}}
            }),
            "tovli_query" => json!({"type":"object","required":["results"],
                "properties":{"results":{"type":"array","items":{"type":"object"}}}}),
            "tovli_link_image" => json!({"type":"object",
                "required":["link_id","document_id","image_document_id","asset_id","anchor_state","status"],
                "properties":{"link_id":{"type":"string"},"document_id":{"type":"string"},
                    "image_document_id":{"type":"string"},"asset_id":{"type":"string"},
                    "anchor_state":{"type":"string"},"status":{"enum":["created","unchanged"]}}}),
            "tovli_list_image_links" => {
                json!({"type":"object","required":["results","next_cursor"],
                "properties":{"results":{"type":"array","items":{"type":"object"}},
                    "next_cursor":{"type":["string","null"]}}})
            }
            _ => unreachable!("known tool"),
        };
        definition.output_schema = Some(
            schema(json!({
                "oneOf":[success,{"type":"object","required":["code","retryable"],
                    "properties":{"code":{"type":"string"},"retryable":{"type":"boolean"}}}]
            }))
            .into(),
        );
    }
    definition
}

fn tools() -> Vec<Tool> {
    vec![
        tool(
            "tovli_add_document",
            "Save a text memory under a stable source key.",
            json!({"type":"object","additionalProperties":false,
            "required":["source_key","content"],"properties":{
                "source_key":{"type":"string","pattern":"^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$"},
                "content":{"type":"string"},
                "format":{"type":"string","enum":["markdown","text"],"default":"markdown"},
                "title":{"type":"string","maxLength":256},
                "project":{"type":"string","maxLength":128},
                "tags":{"type":"array","maxItems":32,"items":{"type":"string","maxLength":64}},
                "metadata":{"type":"object"}
            }}),
            false,
        ),
        tool(
            "tovli_query",
            "Find a committed memory by its exact source key.",
            json!({"type":"object","additionalProperties":false,
            "required":["source_key"],"properties":{
                "source_key":{"type":"string","pattern":"^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$"}
            }}),
            true,
        ),
        tool(
            "tovli_add_image",
            "Save an image with its searchable description.",
            json!({"type":"object","additionalProperties":false,"required":["source_key","image_base64","mime_type","description"],
                "properties":{"source_key":{"type":"string"},"image_base64":{"type":"string"},
                "mime_type":{"type":"string"},"description":{"type":"string"},"ocr_text":{"type":"string"},
                "context":{"type":"string"},"field_provenance":{"type":"object"},"title":{"type":"string"},
                "project":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}}}}),
            false,
        ),
        tool(
            "tovli_get_image",
            "Retrieve an original image by its asset ID.",
            json!({"type":"object","additionalProperties":false,"required":["asset_id"],
                "properties":{"asset_id":{"type":"string"}}}),
            true,
        ),
        tool(
            "tovli_link_image",
            "Link a text document to an image memory.",
            json!({"type":"object","additionalProperties":false,"required":["document_id","image_document_id"],
                "properties":{"document_id":{"type":"string"},"image_document_id":{"type":"string"},
                "chunk_id":{"type":"string"},"caption":{"type":"string"}}}),
            false,
        ),
        tool(
            "tovli_list_image_links",
            "List image links for a document.",
            json!({"type":"object","additionalProperties":false,
                "properties":{"document_id":{"type":"string"},"image_document_id":{"type":"string"},
                "project":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}},
                "cursor":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100,"default":20}},
                "oneOf":[{"required":["document_id"]},{"required":["image_document_id"]}]}),
            true,
        ),
    ]
}

fn valid_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: tools(),
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let mut arguments = Value::Object(request.arguments.unwrap_or_default());
        if request.name.as_ref() == "tovli_frame_limit" {
            return Err(ErrorData::invalid_params("request exceeds limit", None));
        }
        if request.name.as_ref() != "tovli_add_image"
            && serde_json::to_vec(&arguments)
                .map_err(|_| ErrorData::invalid_params("invalid arguments", None))?
                .len()
                > MAX_TEXT_REQUEST
        {
            return Err(ErrorData::invalid_params("request exceeds limit", None));
        }
        let mut image_permit = None;
        let work = match request.name.as_ref() {
            "tovli_add_document" => {
                let input: durable::Input = serde_json::from_value(arguments)
                    .map_err(|_| ErrorData::invalid_params("invalid document arguments", None))?;
                if !valid_key(&input.source_key)
                    || input.content.trim().is_empty()
                    || input.content.len() > 256 * 1024
                    || !matches!(input.format.as_str(), "markdown" | "text")
                    || input
                        .title
                        .as_ref()
                        .is_some_and(|s| s.chars().count() > 256)
                    || input
                        .project
                        .as_ref()
                        .is_some_and(|s| s.chars().count() > 128)
                    || input.tags.len() > 32
                    || input
                        .tags
                        .iter()
                        .any(|t| t.trim().is_empty() || t.chars().count() > 64)
                    || serde_json::to_vec(&input.metadata)
                        .unwrap_or_default()
                        .len()
                        > 4096
                {
                    return Err(ErrorData::invalid_params(
                        "document arguments exceed allowed limits",
                        None,
                    ));
                }
                Work::Add(input)
            }
            "tovli_query" => {
                let input: QueryInput = serde_json::from_value(arguments)
                    .map_err(|_| ErrorData::invalid_params("invalid query arguments", None))?;
                if !valid_key(&input.source_key) {
                    return Err(ErrorData::invalid_params("invalid source key", None));
                }
                Work::Query(input.source_key)
            }
            "tovli_add_image" => {
                let encoded = arguments
                    .as_object_mut()
                    .and_then(|obj| obj.remove("image_base64"))
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .ok_or_else(|| ErrorData::invalid_params("image_base64 required", None))?;
                if encoded.len() > (8_usize * 1024 * 1024).div_ceil(3) * 4 {
                    return Err(ErrorData::invalid_params("image exceeds limit", None));
                }
                let budget = (encoded.len() / 4 * 3).min(8 * 1024 * 1024);
                image_permit = match self
                    .image_budget
                    .clone()
                    .try_acquire_many_owned(budget as u32)
                {
                    Ok(permit) => Some(permit),
                    Err(_) => {
                        return Ok(CallToolResult::structured_error(
                            json!({"code":"busy","retryable":true}),
                        ))
                    }
                };
                let bytes = STANDARD
                    .decode(encoded)
                    .map_err(|_| ErrorData::invalid_params("invalid base64", None))?;
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err(ErrorData::invalid_params("image exceeds limit", None));
                }
                let input: durable::ImageInput = serde_json::from_value(arguments)
                    .map_err(|_| ErrorData::invalid_params("invalid image arguments", None))?;
                let text_size = input.description.len()
                    + input.ocr_text.as_deref().unwrap_or_default().len()
                    + input.context.as_deref().unwrap_or_default().len();
                if !valid_key(&input.source_key)
                    || input.description.trim().is_empty()
                    || text_size > 256 * 1024
                    || input
                        .title
                        .as_ref()
                        .is_some_and(|s| s.chars().count() > 256)
                    || input
                        .project
                        .as_ref()
                        .is_some_and(|s| s.chars().count() > 128)
                    || input.tags.len() > 32
                    || input
                        .tags
                        .iter()
                        .any(|t| t.trim().is_empty() || t.chars().count() > 64)
                    || input
                        .field_provenance
                        .as_ref()
                        .is_some_and(|v| !v.is_object())
                {
                    return Err(ErrorData::invalid_params(
                        "image arguments exceed allowed limits",
                        None,
                    ));
                }
                Work::AddImage(input, bytes)
            }
            "tovli_get_image" => {
                let input: Map<String, Value> = serde_json::from_value(arguments)
                    .map_err(|_| ErrorData::invalid_params("invalid arguments", None))?;
                if input.len() != 1 {
                    return Err(ErrorData::invalid_params("invalid arguments", None));
                }
                Work::GetImage(
                    input
                        .get("asset_id")
                        .and_then(Value::as_str)
                        .ok_or_else(|| ErrorData::invalid_params("asset_id required", None))?
                        .into(),
                )
            }
            "tovli_link_image" => {
                if arguments.as_object().is_some_and(|map| {
                    map.keys().any(|key| {
                        !["document_id", "image_document_id", "chunk_id", "caption"]
                            .contains(&key.as_str())
                    })
                }) {
                    return Err(ErrorData::invalid_params("unknown link argument", None));
                }
                let input: links::LinkInput = serde_json::from_value(arguments)
                    .map_err(|_| ErrorData::invalid_params("invalid link arguments", None))?;
                Work::Link(input)
            }
            "tovli_list_image_links" => {
                let input: links::ListInput = serde_json::from_value(arguments)
                    .map_err(|_| ErrorData::invalid_params("invalid arguments", None))?;
                if input.document_id.is_some() == input.image_document_id.is_some()
                    || input.limit == 0
                    || input.limit > 100
                {
                    return Err(ErrorData::invalid_params(
                        "one endpoint and a limit between 1 and 100 are required",
                        None,
                    ));
                }
                Work::ListLinks(input)
            }
            _ => return Err(ErrorData::invalid_params("unknown tool", None)),
        };
        let (sender, receiver) = oneshot::channel();
        match self.sender.try_send(Job {
            work,
            reply: sender,
            _image_permit: image_permit,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                return Ok(CallToolResult::structured_error(
                    json!({"code":"busy","retryable":true}),
                ))
            }
            Err(TrySendError::Disconnected(_)) => {
                return Ok(CallToolResult::structured_error(
                    json!({"code":"storage_error","retryable":false}),
                ))
            }
        }
        let received = tokio::select! {
            result = receiver => result,
            _ = context.ct.cancelled() => return Err(ErrorData::internal_error("request cancelled", None)),
        };
        let result = match received {
            Ok(Ok(value)) if request.name.as_ref() == "tovli_get_image" => {
                CallToolResult::success(vec![
                    rmcp::model::Content::text(format!(
                        "Asset {}",
                        value["asset_id"].as_str().unwrap_or_default()
                    )),
                    rmcp::model::Content::image(
                        value["image_base64"].as_str().unwrap_or_default(),
                        value["mime_type"].as_str().unwrap_or_default(),
                    ),
                ])
            }
            Ok(Ok(value)) => CallToolResult::structured(value),
            Ok(Err(error)) => {
                let code = [
                    "source_conflict",
                    "embedding_model_mismatch",
                    "store_in_use",
                    "recovery_required",
                    "asset_not_found",
                    "asset_missing",
                    "invalid_image_size",
                    "invalid_image_dimensions",
                    "invalid_image",
                    "unsupported_image_type",
                    "image_mime_mismatch",
                    "link_conflict",
                    "link_inactive",
                    "document_not_found",
                    "invalid_document_kind",
                    "invalid_anchor",
                    "invalid_caption",
                    "invalid_cursor",
                ]
                .into_iter()
                .find(|code| error.contains(code))
                .unwrap_or("storage_error");
                CallToolResult::structured_error(
                    json!({"code":code,"retryable":code == "store_in_use"}),
                )
            }
            Err(_) => {
                CallToolResult::structured_error(json!({"code":"storage_error","retryable":false}))
            }
        };
        Ok(result)
    }
}

// Bound each newline-delimited MCP frame before it is handed to the SDK codec.
async fn bounded_input(mut sink: tokio::io::DuplexStream) -> std::io::Result<()> {
    let mut input = BufReader::new(tokio::io::stdin());
    let mut frame = Vec::new();
    loop {
        let available = input.fill_buf().await?;
        if available.is_empty() {
            return Ok(());
        }
        let end = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |n| n + 1);
        if frame.len() + end > MAX_FRAME {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "MCP frame exceeds limit",
            ));
        }
        frame.extend_from_slice(&available[..end]);
        let complete = available[end - 1] == b'\n';
        input.consume(end);
        if complete {
            if frame.len() > MAX_TEXT_REQUEST {
                let message: Value = serde_json::from_slice(&frame).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid oversized frame")
                })?;
                if message.pointer("/params/name").and_then(Value::as_str)
                    != Some("tovli_add_image")
                {
                    if message.get("method").and_then(Value::as_str) != Some("tools/call") {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "MCP frame exceeds limit",
                        ));
                    }
                    let id = message.get("id").cloned().ok_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "oversized notification",
                        )
                    })?;
                    let replacement = json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
                        "params":{"name":"tovli_frame_limit","arguments":{}}});
                    sink.write_all(replacement.to_string().as_bytes()).await?;
                    sink.write_all(b"\n").await?;
                } else {
                    sink.write_all(&frame).await?;
                }
            } else {
                sink.write_all(&frame).await?;
            }
            frame.clear();
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if !args.store_dir.is_absolute() {
        anyhow::bail!("--store-dir must be an absolute path");
    }
    let (sender, receiver) = sync_channel(16);
    std::thread::spawn(move || worker(&args.store_dir, args.mock, receiver));
    let (client, server) = tokio::io::duplex(64 * 1024);
    let input = tokio::spawn(bounded_input(client));
    let service = Server {
        sender,
        image_budget: Arc::new(Semaphore::new(32 * 1024 * 1024)),
    }
    .serve((server, tokio::io::stdout()))
    .await?;
    service.waiting().await?;
    input.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_queued_work_does_not_touch_the_store() {
        let root = std::env::temp_dir().join(format!("tovli-cancelled-{}", std::process::id()));
        let (sender, receiver) = sync_channel(16);
        let (reply, cancelled) = oneshot::channel();
        drop(cancelled);
        sender
            .send(Job {
                work: Work::Query("cancelled".to_string()),
                reply,
                _image_permit: None,
            })
            .unwrap();
        drop(sender);
        worker(&root, true, receiver);
        assert!(!root.exists());
    }

    #[test]
    fn queued_images_hold_a_bounded_byte_budget() {
        let budget = Arc::new(Semaphore::new(32));
        let (sender, receiver) = sync_channel(16);
        for _ in 0..16 {
            let permit = budget.clone().try_acquire_many_owned(2).unwrap();
            let (reply, _) = oneshot::channel();
            sender
                .try_send(Job {
                    work: Work::Query("key".to_string()),
                    reply,
                    _image_permit: Some(permit),
                })
                .unwrap();
        }
        assert!(budget.clone().try_acquire_many_owned(1).is_err());
        let (reply, _) = oneshot::channel();
        assert!(matches!(
            sender.try_send(Job {
                work: Work::Query("key".to_string()),
                reply,
                _image_permit: None,
            }),
            Err(TrySendError::Full(_))
        ));
        drop(receiver.recv().unwrap());
        assert!(budget.try_acquire_many_owned(2).is_ok());
    }
}
