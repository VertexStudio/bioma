use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct AnnotatedAnnotations {
    #[doc = " Describes who the intended customer of this object or data is."]
    #[doc = " "]
    #[doc = " It can include multiple entries to indicate content useful for multiple audiences (e.g., "]
    #[doc = " `[\"user\", \"assistant\"]`)."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<Role>>,
    #[doc = " Describes how important this data is for operating the server."]
    #[doc = " "]
    #[doc = " A value of 1 means \"most important,\" and indicates that the data is"]
    #[doc = " effectively required, while 0 means \"least important,\" and indicates that"]
    #[doc = " the data is entirely optional."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
}
#[doc = " Base for objects that include optional annotations for the client. The client can use "]
#[doc = " annotations to inform how objects are used or displayed"]
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct Annotated {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<AnnotatedAnnotations>,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct BlobResourceContents {
    #[doc = " A base64-encoded string representing the binary data of the item."]
    pub blob: String,
    #[doc = " The MIME type of this resource, if known."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    #[doc = " The URI of this resource."]
    pub uri: String,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CallToolRequestParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    pub name: String,
}
#[doc = " Used by the client to invoke a tool provided by the server."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CallToolRequest {
    pub method: String,
    pub params: CallToolRequestParams,
}
#[doc = " The server's response to a tool call."]
#[doc = " "]
#[doc = " Any errors that originate from the tool SHOULD be reported inside the result"]
#[doc = " object, with `isError` set to true, _not_ as an MCP protocol-level error"]
#[doc = " response. Otherwise, the LLM would not be able to see that an error occurred"]
#[doc = " and self-correct."]
#[doc = " "]
#[doc = " However, any errors in _finding_ the tool, an error indicating that the"]
#[doc = " server does not support tool calls, or any other exceptional conditions,"]
#[doc = " should be reported as an MCP error response."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CallToolResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    pub content: Vec<serde_json::Value>,
    #[doc = " Whether the tool call ended in an error."]
    #[doc = " "]
    #[doc = " If not set, this is assumed to be false (the call was successful)."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isError")]
    pub is_error: Option<bool>,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CancelledNotificationParams {
    #[doc = " An optional string describing the reason for the cancellation. This MAY be logged or "]
    #[doc = " presented to the user."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[doc = " The ID of the request to cancel."]
    #[doc = " "]
    #[doc = " This MUST correspond to the ID of a request previously issued in the same direction."]
    #[serde(rename = "requestId")]
    pub request_id: RequestId,
}
#[doc = " This notification can be sent by either side to indicate that it is cancelling a "]
#[doc = " previously-issued request."]
#[doc = " "]
#[doc = " The request SHOULD still be in-flight, but due to communication latency, it is always possible "]
#[doc = " that this notification MAY arrive after the request has already finished."]
#[doc = " "]
#[doc = " This notification indicates that the result will be unused, so any associated processing SHOULD "]
#[doc = " cease."]
#[doc = " "]
#[doc = " A client MUST NOT attempt to cancel its `initialize` request."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CancelledNotification {
    pub method: String,
    pub params: CancelledNotificationParams,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ClientCapabilitiesRoots {
    #[doc = " Whether the client supports notifications for changes to the roots list."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "listChanged")]
    pub list_changed: Option<bool>,
}
#[doc = " Capabilities a client may support. Known capabilities are defined here, in this schema, but "]
#[doc = " this is not a closed set: any client can define its own, additional capabilities."]
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ClientCapabilities {
    #[doc = " Experimental, non-standard capabilities that the client supports."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<
        ::std::collections::BTreeMap<
            String,
            ::std::collections::BTreeMap<String, serde_json::Value>,
        >,
    >,
    #[doc = " Present if the client supports listing roots."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<ClientCapabilitiesRoots>,
    #[doc = " Present if the client supports sampling from an LLM."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
}
pub type ClientNotification = serde_json::Value;
pub type ClientRequest = serde_json::Value;
pub type ClientResult = serde_json::Value;
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CompleteRequestParamsArgument {
    #[doc = " The name of the argument"]
    pub name: String,
    #[doc = " The value of the argument to use for completion matching."]
    pub value: String,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CompleteRequestParams {
    #[doc = " The argument's information"]
    pub argument: CompleteRequestParamsArgument,
    #[serde(rename = "ref")]
    pub ref_: serde_json::Value,
}
#[doc = " A request from the client to the server, to ask for completion options."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CompleteRequest {
    pub method: String,
    pub params: CompleteRequestParams,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CompleteResultCompletion {
    #[doc = " Indicates whether there are additional completion options beyond those provided in the "]
    #[doc = " current response, even if the exact total is unknown."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "hasMore")]
    pub has_more: Option<bool>,
    #[doc = " The total number of completion options available. This can exceed the number of values "]
    #[doc = " actually sent in the response."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,
    #[doc = " An array of completion values. Must not exceed 100 items."]
    pub values: Vec<String>,
}
#[doc = " The server's response to a completion/complete request"]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CompleteResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    pub completion: CompleteResultCompletion,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CreateMessageRequestParams {
    #[doc = " A request to include context from one or more MCP servers (including the caller), to be "]
    #[doc = " attached to the prompt. The client MAY ignore this request."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "includeContext")]
    pub include_context: Option<String>,
    #[doc = " The maximum number of tokens to sample, as requested by the server. The client MAY choose "]
    #[doc = " to sample fewer tokens than requested."]
    #[serde(rename = "maxTokens")]
    pub max_tokens: i64,
    pub messages: Vec<SamplingMessage>,
    #[doc = " Optional metadata to pass through to the LLM provider. The format of this metadata is "]
    #[doc = " provider-specific."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    #[doc = " The server's preferences for which model to select. The client MAY ignore these "]
    #[doc = " preferences."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "modelPreferences")]
    pub model_preferences: Option<ModelPreferences>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "stopSequences")]
    pub stop_sequences: Option<Vec<String>>,
    #[doc = " An optional system prompt the server wants to use for sampling. The client MAY modify or "]
    #[doc = " omit this prompt."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "systemPrompt")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}
#[doc = " A request from the server to sample an LLM via the client. The client has full discretion over "]
#[doc = " which model to select. The client should also inform the user before beginning sampling, to "]
#[doc = " allow them to inspect the request (human in the loop) and decide whether to approve it."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CreateMessageRequest {
    pub method: String,
    pub params: CreateMessageRequestParams,
}
#[doc = " The client's response to a sampling/create_message request from the server. The client should "]
#[doc = " inform the user before returning the sampled message, to allow them to inspect the response "]
#[doc = " (human in the loop) and decide whether to allow the server to see it."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct CreateMessageResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    pub content: serde_json::Value,
    #[doc = " The name of the model that generated the message."]
    pub model: String,
    pub role: Role,
    #[doc = " The reason why sampling stopped, if known."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "stopReason")]
    pub stop_reason: Option<String>,
}
#[doc = " An opaque token used to represent a cursor for pagination."]
pub type Cursor = String;
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct EmbeddedResourceAnnotations {
    #[doc = " Describes who the intended customer of this object or data is."]
    #[doc = " "]
    #[doc = " It can include multiple entries to indicate content useful for multiple audiences (e.g., "]
    #[doc = " `[\"user\", \"assistant\"]`)."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<Role>>,
    #[doc = " Describes how important this data is for operating the server."]
    #[doc = " "]
    #[doc = " A value of 1 means \"most important,\" and indicates that the data is"]
    #[doc = " effectively required, while 0 means \"least important,\" and indicates that"]
    #[doc = " the data is entirely optional."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
}
#[doc = " The contents of a resource, embedded into a prompt or tool call result."]
#[doc = " "]
#[doc = " It is up to the client how best to render embedded resources for the benefit"]
#[doc = " of the LLM and/or the user."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct EmbeddedResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<EmbeddedResourceAnnotations>,
    pub resource: serde_json::Value,
    #[serde(rename = "type")]
    pub type_: String,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct EmptyResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct GetPromptRequestParams {
    #[doc = " Arguments to use for templating the prompt."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<::std::collections::BTreeMap<String, String>>,
    #[doc = " The name of the prompt or prompt template."]
    pub name: String,
}
#[doc = " Used by the client to get a prompt provided by the server."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct GetPromptRequest {
    pub method: String,
    pub params: GetPromptRequestParams,
}
#[doc = " The server's response to a prompts/get request from the client."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct GetPromptResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    #[doc = " An optional description for the prompt."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub messages: Vec<PromptMessage>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ImageContentAnnotations {
    #[doc = " Describes who the intended customer of this object or data is."]
    #[doc = " "]
    #[doc = " It can include multiple entries to indicate content useful for multiple audiences (e.g., "]
    #[doc = " `[\"user\", \"assistant\"]`)."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<Role>>,
    #[doc = " Describes how important this data is for operating the server."]
    #[doc = " "]
    #[doc = " A value of 1 means \"most important,\" and indicates that the data is"]
    #[doc = " effectively required, while 0 means \"least important,\" and indicates that"]
    #[doc = " the data is entirely optional."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
}
#[doc = " An image provided to or from an LLM."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ImageContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ImageContentAnnotations>,
    #[doc = " The base64-encoded image data."]
    pub data: String,
    #[doc = " The MIME type of the image. Different providers may support different image types."]
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "type")]
    pub type_: String,
}
#[doc = " Describes the name and version of an MCP implementation."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct Implementation {
    pub name: String,
    pub version: String,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct InitializeRequestParams {
    pub capabilities: ClientCapabilities,
    #[serde(rename = "clientInfo")]
    pub client_info: Implementation,
    #[doc = " The latest version of the Model Context Protocol that the client supports. The client MAY "]
    #[doc = " decide to support older versions as well."]
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
}
#[doc = " This request is sent from the client to the server when it first connects, asking it to begin "]
#[doc = " initialization."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct InitializeRequest {
    pub method: String,
    pub params: InitializeRequestParams,
}
#[doc = " After receiving an initialize request from the client, the server sends this response."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct InitializeResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    pub capabilities: ServerCapabilities,
    #[doc = " Instructions describing how to use the server and its features."]
    #[doc = " "]
    #[doc = " This can be used by clients to improve the LLM's understanding of available tools, "]
    #[doc = " resources, etc. It can be thought of like a \"hint\" to the model. For example, this "]
    #[doc = " information MAY be added to the system prompt."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[doc = " The version of the Model Context Protocol that the server wants to use. This may not match "]
    #[doc = " the version that the client requested. If the client cannot support this version, it MUST "]
    #[doc = " disconnect."]
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(rename = "serverInfo")]
    pub server_info: Implementation,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct InitializedNotificationParams {
    #[doc = " This parameter name is reserved by MCP to allow clients and servers to attach additional "]
    #[doc = " metadata to their notifications."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
}
#[doc = " This notification is sent from the client to the server after initialization has finished."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct InitializedNotification {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<InitializedNotificationParams>,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct JsonrpcerrorError {
    #[doc = " The error type that occurred."]
    pub code: i64,
    #[doc = " Additional information about the error. The value of this member is defined by the sender "]
    #[doc = " (e.g. detailed error information, nested errors etc.)."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[doc = " A short description of the error. The message SHOULD be limited to a concise single "]
    #[doc = " sentence."]
    pub message: String,
}
#[doc = " A response to a request that indicates an error occurred."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(rename = "JSONRPCError")]
pub struct Jsonrpcerror {
    pub error: JsonrpcerrorError,
    pub id: RequestId,
    pub jsonrpc: String,
}
pub type Jsonrpcmessage = serde_json::Value;
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct JsonrpcnotificationParams {
    #[doc = " This parameter name is reserved by MCP to allow clients and servers to attach additional "]
    #[doc = " metadata to their notifications."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
}
#[doc = " A notification which does not expect a response."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(rename = "JSONRPCNotification")]
pub struct Jsonrpcnotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<JsonrpcnotificationParams>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct JsonrpcrequestParamsMeta {
    #[doc = " If specified, the caller is requesting out-of-band progress notifications for this request "]
    #[doc = " (as represented by notifications/progress). The value of this parameter is an opaque token "]
    #[doc = " that will be attached to any subsequent notifications. The receiver is not obligated to "]
    #[doc = " provide these notifications."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "progressToken")]
    pub progress_token: Option<ProgressToken>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct JsonrpcrequestParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<JsonrpcrequestParamsMeta>,
}
#[doc = " A request that expects a response."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(rename = "JSONRPCRequest")]
pub struct Jsonrpcrequest {
    pub id: RequestId,
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<JsonrpcrequestParams>,
}
#[doc = " A successful (non-error) response to a request."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
#[serde(rename = "JSONRPCResponse")]
pub struct Jsonrpcresponse {
    pub id: RequestId,
    pub jsonrpc: String,
    pub result: Result,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ListPromptsRequestParams {
    #[doc = " An opaque token representing the current pagination position."]
    #[doc = " If provided, the server should return results starting after this cursor."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}
#[doc = " Sent from the client to request a list of prompts and prompt templates the server has."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ListPromptsRequest {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<ListPromptsRequestParams>,
}
#[doc = " The server's response to a prompts/list request from the client."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ListPromptsResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    #[doc = " An opaque token representing the pagination position after the last returned result."]
    #[doc = " If present, there may be more results available."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "nextCursor")]
    pub next_cursor: Option<String>,
    pub prompts: Vec<Prompt>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ListResourceTemplatesRequestParams {
    #[doc = " An opaque token representing the current pagination position."]
    #[doc = " If provided, the server should return results starting after this cursor."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}
#[doc = " Sent from the client to request a list of resource templates the server has."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ListResourceTemplatesRequest {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<ListResourceTemplatesRequestParams>,
}
#[doc = " The server's response to a resources/templates/list request from the client."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ListResourceTemplatesResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    #[doc = " An opaque token representing the pagination position after the last returned result."]
    #[doc = " If present, there may be more results available."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "nextCursor")]
    pub next_cursor: Option<String>,
    #[serde(rename = "resourceTemplates")]
    pub resource_templates: Vec<ResourceTemplate>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ListResourcesRequestParams {
    #[doc = " An opaque token representing the current pagination position."]
    #[doc = " If provided, the server should return results starting after this cursor."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}
#[doc = " Sent from the client to request a list of resources the server has."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ListResourcesRequest {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<ListResourcesRequestParams>,
}
#[doc = " The server's response to a resources/list request from the client."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ListResourcesResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    #[doc = " An opaque token representing the pagination position after the last returned result."]
    #[doc = " If present, there may be more results available."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "nextCursor")]
    pub next_cursor: Option<String>,
    pub resources: Vec<Resource>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ListRootsRequestParamsMeta {
    #[doc = " If specified, the caller is requesting out-of-band progress notifications for this request "]
    #[doc = " (as represented by notifications/progress). The value of this parameter is an opaque token "]
    #[doc = " that will be attached to any subsequent notifications. The receiver is not obligated to "]
    #[doc = " provide these notifications."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "progressToken")]
    pub progress_token: Option<ProgressToken>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ListRootsRequestParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<ListRootsRequestParamsMeta>,
}
#[doc = " Sent from the server to request a list of root URIs from the client. Roots allow"]
#[doc = " servers to ask for specific directories or files to operate on. A common example"]
#[doc = " for roots is providing a set of repositories or directories a server should operate"]
#[doc = " on."]
#[doc = " "]
#[doc = " This request is typically used when the server needs to understand the file system"]
#[doc = " structure or access specific locations that the client has permission to read from."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ListRootsRequest {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<ListRootsRequestParams>,
}
#[doc = " The client's response to a roots/list request from the server."]
#[doc = " This result contains an array of Root objects, each representing a root directory"]
#[doc = " or file that the server can operate on."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ListRootsResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    pub roots: Vec<Root>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ListToolsRequestParams {
    #[doc = " An opaque token representing the current pagination position."]
    #[doc = " If provided, the server should return results starting after this cursor."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}
#[doc = " Sent from the client to request a list of tools the server has."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ListToolsRequest {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<ListToolsRequestParams>,
}
#[doc = " The server's response to a tools/list request from the client."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ListToolsResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    #[doc = " An opaque token representing the pagination position after the last returned result."]
    #[doc = " If present, there may be more results available."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "nextCursor")]
    pub next_cursor: Option<String>,
    pub tools: Vec<Tool>,
}
#[doc = " The severity of a log message."]
#[doc = " "]
#[doc = " These map to syslog message severities, as specified in RFC-5424:"]
#[doc = " https://datatracker.ietf.org/doc/html/rfc5424#section-6.2.1"]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub enum LoggingLevel {
    #[serde(rename = "alert")]
    Alert,
    #[serde(rename = "critical")]
    Critical,
    #[serde(rename = "debug")]
    Debug,
    #[serde(rename = "emergency")]
    Emergency,
    #[serde(rename = "error")]
    Error,
    #[serde(rename = "info")]
    Info,
    #[serde(rename = "notice")]
    Notice,
    #[serde(rename = "warning")]
    Warning,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct LoggingMessageNotificationParams {
    #[doc = " The data to be logged, such as a string message or an object. Any JSON serializable type is "]
    #[doc = " allowed here."]
    pub data: serde_json::Value,
    #[doc = " The severity of this log message."]
    pub level: LoggingLevel,
    #[doc = " An optional name of the logger issuing this message."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logger: Option<String>,
}
#[doc = " Notification of a log message passed from server to client. If no logging/setLevel request has "]
#[doc = " been sent from the client, the server MAY decide which messages to send automatically."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct LoggingMessageNotification {
    pub method: String,
    pub params: LoggingMessageNotificationParams,
}
#[doc = " Hints to use for model selection."]
#[doc = " "]
#[doc = " Keys not declared here are currently left unspecified by the spec and are up"]
#[doc = " to the client to interpret."]
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ModelHint {
    #[doc = " A hint for a model name."]
    #[doc = " "]
    #[doc = " The client SHOULD treat this as a substring of a model name; for example:"]
    #[doc = "  - `claude-3-5-sonnet` should match `claude-3-5-sonnet-20241022`"]
    #[doc = "  - `sonnet` should match `claude-3-5-sonnet-20241022`, `claude-3-sonnet-20240229`, etc."]
    #[doc = "  - `claude` should match any Claude model"]
    #[doc = " "]
    #[doc = " The client MAY also map the string to a different provider's model name or a different "]
    #[doc = " model family, as long as it fills a similar niche; for example:"]
    #[doc = "  - `gemini-1.5-flash` could match `claude-3-haiku-20240307`"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}
#[doc = " The server's preferences for model selection, requested of the client during sampling."]
#[doc = " "]
#[doc = " Because LLMs can vary along multiple dimensions, choosing the \"best\" model is"]
#[doc = " rarely straightforward.  Different models excel in different areas—some are"]
#[doc = " faster but less capable, others are more capable but more expensive, and so"]
#[doc = " on. This interface allows servers to express their priorities across multiple"]
#[doc = " dimensions to help clients make an appropriate selection for their use case."]
#[doc = " "]
#[doc = " These preferences are always advisory. The client MAY ignore them. It is also"]
#[doc = " up to the client to decide how to interpret these preferences and how to"]
#[doc = " balance them against other considerations."]
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ModelPreferences {
    #[doc = " How much to prioritize cost when selecting a model. A value of 0 means cost"]
    #[doc = " is not important, while a value of 1 means cost is the most important"]
    #[doc = " factor."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "costPriority")]
    pub cost_priority: Option<f64>,
    #[doc = " Optional hints to use for model selection."]
    #[doc = " "]
    #[doc = " If multiple hints are specified, the client MUST evaluate them in order"]
    #[doc = " (such that the first match is taken)."]
    #[doc = " "]
    #[doc = " The client SHOULD prioritize these hints over the numeric priorities, but"]
    #[doc = " MAY still use the priorities to select from ambiguous matches."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints: Option<Vec<ModelHint>>,
    #[doc = " How much to prioritize intelligence and capabilities when selecting a"]
    #[doc = " model. A value of 0 means intelligence is not important, while a value of 1"]
    #[doc = " means intelligence is the most important factor."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "intelligencePriority")]
    pub intelligence_priority: Option<f64>,
    #[doc = " How much to prioritize sampling speed (latency) when selecting a model. A"]
    #[doc = " value of 0 means speed is not important, while a value of 1 means speed is"]
    #[doc = " the most important factor."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "speedPriority")]
    pub speed_priority: Option<f64>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct NotificationParams {
    #[doc = " This parameter name is reserved by MCP to allow clients and servers to attach additional "]
    #[doc = " metadata to their notifications."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct Notification {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<NotificationParams>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct PaginatedRequestParams {
    #[doc = " An opaque token representing the current pagination position."]
    #[doc = " If provided, the server should return results starting after this cursor."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct PaginatedRequest {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<PaginatedRequestParams>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct PaginatedResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    #[doc = " An opaque token representing the pagination position after the last returned result."]
    #[doc = " If present, there may be more results available."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "nextCursor")]
    pub next_cursor: Option<String>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct PingRequestParamsMeta {
    #[doc = " If specified, the caller is requesting out-of-band progress notifications for this request "]
    #[doc = " (as represented by notifications/progress). The value of this parameter is an opaque token "]
    #[doc = " that will be attached to any subsequent notifications. The receiver is not obligated to "]
    #[doc = " provide these notifications."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "progressToken")]
    pub progress_token: Option<ProgressToken>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct PingRequestParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<PingRequestParamsMeta>,
}
#[doc = " A ping, issued by either the server or the client, to check that the other party is still "]
#[doc = " alive. The receiver must promptly respond, or else may be disconnected."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct PingRequest {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<PingRequestParams>,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ProgressNotificationParams {
    #[doc = " The progress thus far. This should increase every time progress is made, even if the total "]
    #[doc = " is unknown."]
    pub progress: f64,
    #[doc = " The progress token which was given in the initial request, used to associate this "]
    #[doc = " notification with the request that is proceeding."]
    #[serde(rename = "progressToken")]
    pub progress_token: ProgressToken,
    #[doc = " Total number of items to process (or total progress required), if known."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
}
#[doc = " An out-of-band notification used to inform the receiver of a progress update for a long-running "]
#[doc = " request."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ProgressNotification {
    pub method: String,
    pub params: ProgressNotificationParams,
}
#[doc = " A progress token, used to associate progress notifications with the original request."]
pub type ProgressToken = serde_json::Value;
#[doc = " A prompt or prompt template that the server offers."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct Prompt {
    #[doc = " A list of arguments to use for templating the prompt."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Vec<PromptArgument>>,
    #[doc = " An optional description of what this prompt provides"]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[doc = " The name of the prompt or prompt template."]
    pub name: String,
}
#[doc = " Describes an argument that a prompt can accept."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct PromptArgument {
    #[doc = " A human-readable description of the argument."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[doc = " The name of the argument."]
    pub name: String,
    #[doc = " Whether this argument must be provided."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct PromptListChangedNotificationParams {
    #[doc = " This parameter name is reserved by MCP to allow clients and servers to attach additional "]
    #[doc = " metadata to their notifications."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
}
#[doc = " An optional notification from the server to the client, informing it that the list of prompts "]
#[doc = " it offers has changed. This may be issued by servers without any previous subscription from the "]
#[doc = " client."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct PromptListChangedNotification {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<PromptListChangedNotificationParams>,
}
#[doc = " Describes a message returned as part of a prompt."]
#[doc = " "]
#[doc = " This is similar to `SamplingMessage`, but also supports the embedding of"]
#[doc = " resources from the MCP server."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct PromptMessage {
    pub content: serde_json::Value,
    pub role: Role,
}
#[doc = " Identifies a prompt."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct PromptReference {
    #[doc = " The name of the prompt or prompt template"]
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ReadResourceRequestParams {
    #[doc = " The URI of the resource to read. The URI can use any protocol; it is up to the server how "]
    #[doc = " to interpret it."]
    pub uri: String,
}
#[doc = " Sent from the client to the server, to read a specific resource URI."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ReadResourceRequest {
    pub method: String,
    pub params: ReadResourceRequestParams,
}
#[doc = " The server's response to a resources/read request from the client."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ReadResourceResult {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    pub contents: Vec<serde_json::Value>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct RequestParamsMeta {
    #[doc = " If specified, the caller is requesting out-of-band progress notifications for this request "]
    #[doc = " (as represented by notifications/progress). The value of this parameter is an opaque token "]
    #[doc = " that will be attached to any subsequent notifications. The receiver is not obligated to "]
    #[doc = " provide these notifications."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "progressToken")]
    pub progress_token: Option<ProgressToken>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct RequestParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<RequestParamsMeta>,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct Request {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<RequestParams>,
}
#[doc = " A uniquely identifying ID for a request in JSON-RPC."]
pub type RequestId = serde_json::Value;
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ResourceAnnotations {
    #[doc = " Describes who the intended customer of this object or data is."]
    #[doc = " "]
    #[doc = " It can include multiple entries to indicate content useful for multiple audiences (e.g., "]
    #[doc = " `[\"user\", \"assistant\"]`)."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<Role>>,
    #[doc = " Describes how important this data is for operating the server."]
    #[doc = " "]
    #[doc = " A value of 1 means \"most important,\" and indicates that the data is"]
    #[doc = " effectively required, while 0 means \"least important,\" and indicates that"]
    #[doc = " the data is entirely optional."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
}
#[doc = " A known resource that the server is capable of reading."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct Resource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ResourceAnnotations>,
    #[doc = " A description of what this resource represents."]
    #[doc = " "]
    #[doc = " This can be used by clients to improve the LLM's understanding of available resources. It "]
    #[doc = " can be thought of like a \"hint\" to the model."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[doc = " The MIME type of this resource, if known."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    #[doc = " A human-readable name for this resource."]
    #[doc = " "]
    #[doc = " This can be used by clients to populate UI elements."]
    pub name: String,
    #[doc = " The URI of this resource."]
    pub uri: String,
}
#[doc = " The contents of a specific resource or sub-resource."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ResourceContents {
    #[doc = " The MIME type of this resource, if known."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    #[doc = " The URI of this resource."]
    pub uri: String,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ResourceListChangedNotificationParams {
    #[doc = " This parameter name is reserved by MCP to allow clients and servers to attach additional "]
    #[doc = " metadata to their notifications."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
}
#[doc = " An optional notification from the server to the client, informing it that the list of resources "]
#[doc = " it can read from has changed. This may be issued by servers without any previous subscription "]
#[doc = " from the client."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ResourceListChangedNotification {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<ResourceListChangedNotificationParams>,
}
#[doc = " A reference to a resource or resource template definition."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ResourceReference {
    #[serde(rename = "type")]
    pub type_: String,
    #[doc = " The URI or URI template of the resource."]
    pub uri: String,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ResourceTemplateAnnotations {
    #[doc = " Describes who the intended customer of this object or data is."]
    #[doc = " "]
    #[doc = " It can include multiple entries to indicate content useful for multiple audiences (e.g., "]
    #[doc = " `[\"user\", \"assistant\"]`)."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<Role>>,
    #[doc = " Describes how important this data is for operating the server."]
    #[doc = " "]
    #[doc = " A value of 1 means \"most important,\" and indicates that the data is"]
    #[doc = " effectively required, while 0 means \"least important,\" and indicates that"]
    #[doc = " the data is entirely optional."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
}
#[doc = " A template description for resources available on the server."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ResourceTemplate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ResourceTemplateAnnotations>,
    #[doc = " A description of what this template is for."]
    #[doc = " "]
    #[doc = " This can be used by clients to improve the LLM's understanding of available resources. It "]
    #[doc = " can be thought of like a \"hint\" to the model."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[doc = " The MIME type for all resources that match this template. This should only be included if "]
    #[doc = " all resources matching this template have the same type."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    #[doc = " A human-readable name for the type of resource this template refers to."]
    #[doc = " "]
    #[doc = " This can be used by clients to populate UI elements."]
    pub name: String,
    #[doc = " A URI template (according to RFC 6570) that can be used to construct resource URIs."]
    #[serde(rename = "uriTemplate")]
    pub uri_template: String,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ResourceUpdatedNotificationParams {
    #[doc = " The URI of the resource that has been updated. This might be a sub-resource of the one that "]
    #[doc = " the client actually subscribed to."]
    pub uri: String,
}
#[doc = " A notification from the server to the client, informing it that a resource has changed and may "]
#[doc = " need to be read again. This should only be sent if the client previously sent a "]
#[doc = " resources/subscribe request."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ResourceUpdatedNotification {
    pub method: String,
    pub params: ResourceUpdatedNotificationParams,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct Result {
    #[doc = " This result property is reserved by the protocol to allow clients and servers to attach "]
    #[doc = " additional metadata to their responses."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
}
#[doc = " The sender or recipient of messages and data in a conversation."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub enum Role {
    #[serde(rename = "assistant")]
    Assistant,
    #[serde(rename = "user")]
    User,
}
#[doc = " Represents a root directory or file that the server can operate on."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct Root {
    #[doc = " An optional name for the root. This can be used to provide a human-readable"]
    #[doc = " identifier for the root, which may be useful for display purposes or for"]
    #[doc = " referencing the root in other parts of the application."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[doc = " The URI identifying the root. This *must* start with file:// for now."]
    #[doc = " This restriction may be relaxed in future versions of the protocol to allow"]
    #[doc = " other URI schemes."]
    pub uri: String,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct RootsListChangedNotificationParams {
    #[doc = " This parameter name is reserved by MCP to allow clients and servers to attach additional "]
    #[doc = " metadata to their notifications."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
}
#[doc = " A notification from the client to the server, informing it that the list of roots has changed."]
#[doc = " This notification should be sent whenever the client adds, removes, or modifies any root."]
#[doc = " The server should then request an updated list of roots using the ListRootsRequest."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct RootsListChangedNotification {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<RootsListChangedNotificationParams>,
}
#[doc = " Describes a message issued to or received from an LLM API."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct SamplingMessage {
    pub content: serde_json::Value,
    pub role: Role,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ServerCapabilitiesPrompts {
    #[doc = " Whether this server supports notifications for changes to the prompt list."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "listChanged")]
    pub list_changed: Option<bool>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ServerCapabilitiesPromptsResources {
    #[doc = " Whether this server supports notifications for changes to the resource list."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "listChanged")]
    pub list_changed: Option<bool>,
    #[doc = " Whether this server supports subscribing to resource updates."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<bool>,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ServerCapabilitiesPromptsResourcesTools {
    #[doc = " Whether this server supports notifications for changes to the tool list."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "listChanged")]
    pub list_changed: Option<bool>,
}
#[doc = " Capabilities that a server may support. Known capabilities are defined here, in this schema, "]
#[doc = " but this is not a closed set: any server can define its own, additional capabilities."]
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ServerCapabilities {
    #[doc = " Experimental, non-standard capabilities that the server supports."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<
        ::std::collections::BTreeMap<
            String,
            ::std::collections::BTreeMap<String, serde_json::Value>,
        >,
    >,
    #[doc = " Present if the server supports sending log messages to the client."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
    #[doc = " Present if the server offers any prompt templates."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<ServerCapabilitiesPrompts>,
    #[doc = " Present if the server offers any resources to read."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ServerCapabilitiesPromptsResources>,
    #[doc = " Present if the server offers any tools to call."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ServerCapabilitiesPromptsResourcesTools>,
}
pub type ServerNotification = serde_json::Value;
pub type ServerRequest = serde_json::Value;
pub type ServerResult = serde_json::Value;
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct SetLevelRequestParams {
    #[doc = " The level of logging that the client wants to receive from the server. The server should "]
    #[doc = " send all logs at this level and higher (i.e., more severe) to the client as "]
    #[doc = " notifications/logging/message."]
    pub level: LoggingLevel,
}
#[doc = " A request from the client to the server, to enable or adjust logging."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct SetLevelRequest {
    pub method: String,
    pub params: SetLevelRequestParams,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct SubscribeRequestParams {
    #[doc = " The URI of the resource to subscribe to. The URI can use any protocol; it is up to the "]
    #[doc = " server how to interpret it."]
    pub uri: String,
}
#[doc = " Sent from the client to request resources/updated notifications from the server whenever a "]
#[doc = " particular resource changes."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct SubscribeRequest {
    pub method: String,
    pub params: SubscribeRequestParams,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct TextContentAnnotations {
    #[doc = " Describes who the intended customer of this object or data is."]
    #[doc = " "]
    #[doc = " It can include multiple entries to indicate content useful for multiple audiences (e.g., "]
    #[doc = " `[\"user\", \"assistant\"]`)."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audience: Option<Vec<Role>>,
    #[doc = " Describes how important this data is for operating the server."]
    #[doc = " "]
    #[doc = " A value of 1 means \"most important,\" and indicates that the data is"]
    #[doc = " effectively required, while 0 means \"least important,\" and indicates that"]
    #[doc = " the data is entirely optional."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
}
#[doc = " Text provided to or from an LLM."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct TextContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<TextContentAnnotations>,
    #[doc = " The text content of the message."]
    pub text: String,
    #[serde(rename = "type")]
    pub type_: String,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct TextResourceContents {
    #[doc = " The MIME type of this resource, if known."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    #[doc = " The text of the item. This must only be set if the item can actually be represented as text "]
    #[doc = " (not binary data)."]
    pub text: String,
    #[doc = " The URI of this resource."]
    pub uri: String,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ToolInputSchema {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<
        ::std::collections::BTreeMap<
            String,
            ::std::collections::BTreeMap<String, serde_json::Value>,
        >,
    >,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
    #[serde(rename = "type")]
    pub type_: String,
}
#[doc = " Definition for a tool the client can call."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct Tool {
    #[doc = " A human-readable description of the tool."]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[doc = " A JSON Schema object defining the expected parameters for the tool."]
    #[serde(rename = "inputSchema")]
    pub input_schema: ToolInputSchema,
    #[doc = " The name of the tool."]
    pub name: String,
}
#[derive(Clone, PartialEq, Debug, Default, Deserialize, Serialize)]
pub struct ToolListChangedNotificationParams {
    #[doc = " This parameter name is reserved by MCP to allow clients and servers to attach additional "]
    #[doc = " metadata to their notifications."]
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "_meta")]
    pub meta: Option<::std::collections::BTreeMap<String, serde_json::Value>>,
}
#[doc = " An optional notification from the server to the client, informing it that the list of tools it "]
#[doc = " offers has changed. This may be issued by servers without any previous subscription from the "]
#[doc = " client."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct ToolListChangedNotification {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<ToolListChangedNotificationParams>,
}
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct UnsubscribeRequestParams {
    #[doc = " The URI of the resource to unsubscribe from."]
    pub uri: String,
}
#[doc = " Sent from the client to request cancellation of resources/updated notifications from the "]
#[doc = " server. This should follow a previous resources/subscribe request."]
#[derive(Clone, PartialEq, Debug, Deserialize, Serialize)]
pub struct UnsubscribeRequest {
    pub method: String,
    pub params: UnsubscribeRequestParams,
}
pub type SchemaJson = serde_json::Value;
