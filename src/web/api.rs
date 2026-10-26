//! Defines Rojo's HTTP API, all under /api. These endpoints generally return
//! JSON.

use std::{
    collections::{HashMap, HashSet},
    fs,
    net::SocketAddr,
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};

use futures::{sink::SinkExt, stream::StreamExt};
use hyper::{
    body::{self, HttpBody},
    Body, Method, Request, Response, StatusCode,
};
use hyper_tungstenite::{is_upgrade_request, tungstenite::Message, upgrade, HyperWebsocket};
use opener::OpenError;
use rbx_dom_weak::{
    types::{Ref, Variant},
    InstanceBuilder, UstrMap, WeakDom,
};
use std::num::NonZeroUsize;

use crate::{
    serve_session::ServeSession,
    snapshot::{InstanceWithMeta, PatchSet, PatchUpdate},
    web::{
        class_authority,
        interface::{
            ErrorResponse, Instance, MessagesPacket, OpenResponse, ReadResponse,
            ServerInfoResponse, SocketPacket, SocketPacketBody, SocketPacketType, SubscribeMessage,
            WriteRequest, WriteResponse, PROTOCOL_VERSION, SERVER_VERSION,
        },
        origin::canonical,
        property_removals, source_paths, studio_output, tweeninfo_attributes,
        util::{deserialize_msgpack, msgpack, msgpack_ok},
    },
    web_api::{
        InstanceUpdate, RefPatchRequest, RefPatchResponse, SerializeRequest, SerializeResponse,
    },
};

#[cfg(test)]
async fn call(
    serve_session: Arc<ServeSession>,
    remote_addr: SocketAddr,
    request: Request<Body>,
) -> Response<Body> {
    call_with_body_limit(
        serve_session,
        remote_addr,
        request,
        NonZeroUsize::new(super::DEFAULT_MAX_REQUEST_BODY_BYTES).unwrap(),
    )
    .await
}

#[cfg(test)]
async fn call_with_body_limit(
    serve_session: Arc<ServeSession>,
    remote_addr: SocketAddr,
    request: Request<Body>,
    request_body_limit: NonZeroUsize,
) -> Response<Body> {
    call_with_options(
        serve_session,
        remote_addr,
        request,
        request_body_limit,
        [false; 4],
        None,
    )
    .await
}

pub(super) async fn call_with_options(
    serve_session: Arc<ServeSession>,
    remote_addr: SocketAddr,
    mut request: Request<Body>,
    request_body_limit: NonZeroUsize,
    output_logs: [bool; 4],
    debugger_plans: Option<super::debugger_plans::Shared>,
) -> Response<Body> {
    let mut service = ApiService::new(serve_session, remote_addr);
    service.request_body_limit = request_body_limit;
    service.output_logs = output_logs;
    service.debugger_plans = debugger_plans;

    if request.uri().path() == "/api/write-receipts"
        || request.uri().path().starts_with("/api/write-receipts/")
    {
        return crate::write_receipts::handle(&service.serve_session, request, request_body_limit)
            .await;
    }

    if request.uri().path().starts_with("/api/debugger/") {
        if let Some(broker) = &service.debugger_plans {
            return super::debugger_plans::handle(
                &service.serve_session,
                broker,
                request,
                request_body_limit,
            )
            .await;
        }
    }

    match (request.method(), request.uri().path()) {
        (&Method::GET, "/api/rojo") => service.handle_api_rojo().await,
        (&Method::GET, path) if path.starts_with("/api/read/") => {
            service.handle_api_read(request).await
        }
        (&Method::GET, path) if path.starts_with("/api/socket/") => {
            if is_upgrade_request(&request) {
                service.handle_api_socket(&mut request).await
            } else {
                msgpack(
                    ErrorResponse::bad_request(
                        "/api/socket must be called as a websocket upgrade request",
                    ),
                    StatusCode::BAD_REQUEST,
                )
            }
        }
        (&Method::POST, "/api/serialize") => service.handle_api_serialize(request).await,
        (&Method::POST, "/api/ref-patch") => service.handle_api_ref_patch(request).await,

        (&Method::POST, path) if path.starts_with("/api/open/") => {
            service.handle_api_open(request).await
        }
        (&Method::POST, "/api/output") if output_logs.iter().any(|enabled| *enabled) => {
            service.handle_api_output(request).await
        }
        (&Method::POST, "/api/write") => service.handle_api_write(request).await,

        (_method, path) => msgpack(
            ErrorResponse::not_found(format!("Route not found: {}", path)),
            StatusCode::NOT_FOUND,
        ),
    }
}

pub struct ApiService {
    serve_session: Arc<ServeSession>,
    remote_addr: SocketAddr,
    request_body_limit: NonZeroUsize,
    output_logs: [bool; 4],
    debugger_plans: Option<super::debugger_plans::Shared>,
}

pub(crate) async fn request_bytes(
    request: Request<Body>,
    limit: NonZeroUsize,
) -> Result<body::Bytes, (ErrorResponse, StatusCode)> {
    let route = request.uri().path().to_owned();
    let too_large = || {
        (ErrorResponse::bad_request(format!(
        "Request body for {route} exceeds {} bytes. Increase --max-request-body-bytes when serving intentionally larger requests.", limit.get()
    )), StatusCode::PAYLOAD_TOO_LARGE)
    };
    let declared_length = request
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if declared_length.is_some_and(|length| length > limit.get() as u64)
        || request.body().size_hint().lower() > limit.get() as u64
    {
        return Err(too_large());
    }
    let mut body = request.into_body();
    let mut bytes = Vec::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk.map_err(|err| {
            (
                ErrorResponse::bad_request(format!(
                    "Could not read request body for {route}: {err}"
                )),
                StatusCode::BAD_REQUEST,
            )
        })?;
        // Check before extending: neither an absent/incorrect length header
        // nor multiple streamed chunks may bypass the collection bound.
        if chunk.len() > limit.get() - bytes.len() {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes.into())
}

impl ApiService {
    pub fn new(serve_session: Arc<ServeSession>, remote_addr: SocketAddr) -> Self {
        ApiService {
            serve_session,
            remote_addr,
            request_body_limit: NonZeroUsize::new(super::DEFAULT_MAX_REQUEST_BODY_BYTES).unwrap(),
            output_logs: [false; 4],
            debugger_plans: None,
        }
    }

    /// Get a summary of information about the server
    async fn handle_api_rojo(&self) -> Response<Body> {
        let tree = self.serve_session.tree();
        let root_instance_id = tree.get_root_id();

        #[derive(serde::Serialize)]
        struct InfoWithCapabilities {
            #[serde(flatten)]
            info: ServerInfoResponse,
            capabilities: Vec<&'static str>,
            #[serde(rename = "outputLogs", skip_serializing_if = "Option::is_none")]
            output_logs: Option<Vec<&'static str>>,
            #[serde(rename = "debuggerPlans", skip_serializing_if = "Option::is_none")]
            debugger_plans: Option<serde_json::Value>,
        }
        msgpack_ok(InfoWithCapabilities {
            info: ServerInfoResponse {
                server_version: SERVER_VERSION.to_owned(),
                protocol_version: PROTOCOL_VERSION,
                session_id: self.serve_session.session_id(),
                project_name: self.serve_session.project_name().to_owned(),
                expected_place_ids: self.serve_session.serve_place_ids().cloned(),
                unexpected_place_ids: self.serve_session.blocked_place_ids().cloned(),
                place_id: self.serve_session.place_id(),
                game_id: self.serve_session.game_id(),
                root_instance_id,
            },
            capabilities: {
                let mut capabilities = vec![
                    property_removals::CAPABILITY,
                    tweeninfo_attributes::CAPABILITY,
                    "serializedModelsV1",
                    crate::write_receipts::CAPABILITY,
                    class_authority::CAPABILITY,
                ];
                if self.output_logs.iter().any(|enabled| *enabled) {
                    capabilities.push(studio_output::CAPABILITY);
                }
                if self.debugger_plans.is_some() {
                    capabilities.push(super::debugger_plans::CAPABILITY);
                }
                capabilities
            },
            debugger_plans: self
                .debugger_plans
                .as_ref()
                .map(|_| super::debugger_plans::limits()),
            output_logs: self.output_logs.iter().any(|enabled| *enabled).then(|| {
                studio_output::LEVELS
                    .iter()
                    .zip(self.output_logs)
                    .filter_map(|(level, enabled)| enabled.then_some(*level))
                    .collect()
            }),
        })
    }

    async fn handle_api_output(&self, request: Request<Body>) -> Response<Body> {
        let limit = NonZeroUsize::new(self.request_body_limit.get().min(studio_output::BODY_LIMIT))
            .unwrap();
        let bytes = match request_bytes(request, limit).await {
            Ok(bytes) => bytes,
            Err((error, status)) => {
                return msgpack(
                    if status == StatusCode::PAYLOAD_TOO_LARGE {
                        ErrorResponse::bad_request(format!(
                            "Studio output body exceeds {} bytes (global/per-route limit)",
                            limit.get()
                        ))
                    } else {
                        error
                    },
                    status,
                )
            }
        };
        let batch = match deserialize_msgpack::<studio_output::Batch>(&bytes) {
            Ok(batch) => batch,
            Err(error) => {
                return msgpack(
                    ErrorResponse::bad_request(format!("Invalid Studio output body: {error}")),
                    StatusCode::BAD_REQUEST,
                )
            }
        };
        let lines = match studio_output::format_batch(&self.serve_session, self.output_logs, batch)
        {
            Ok(lines) => lines,
            Err(error) => {
                return msgpack(
                    ErrorResponse::bad_request(error.to_string()),
                    StatusCode::BAD_REQUEST,
                )
            }
        };
        // Lock for the whole validated batch, and handle a closed terminal
        // without panicking the live server.
        use std::io::Write;
        let mut stderr = std::io::stderr().lock();
        for line in lines {
            if let Err(error) = writeln!(stderr, "{line}") {
                return msgpack(
                    ErrorResponse::internal_error(format!(
                        "Could not write Studio output: {error}"
                    )),
                    StatusCode::INTERNAL_SERVER_ERROR,
                );
            }
        }
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Ack {
            session_id: crate::SessionId,
        }
        msgpack_ok(Ack {
            session_id: self.serve_session.session_id(),
        })
    }

    /// Handle WebSocket upgrade for real-time message streaming
    async fn handle_api_socket(&self, request: &mut Request<Body>) -> Response<Body> {
        let authority = class_authority::negotiated(request.uri().query());
        if !authority && class_authority::requires_client(&self.serve_session.tree()) {
            return msgpack(ErrorResponse::bad_request("Studio class authority requires a client with classAuthorityV1; update the plugin"), StatusCode::CONFLICT);
        }
        let tweeninfo_attributes = tweeninfo_attributes::negotiated(request.uri().query());
        let property_removals = request.uri().query().is_some_and(|query| {
            query
                .split('&')
                .any(|parameter| parameter == "propertyRemovalsV1=true")
        });
        let argument = &request.uri().path()["/api/socket/".len()..];
        let input_cursor: u32 = match argument.parse() {
            Ok(v) => v,
            Err(err) => {
                return msgpack(
                    ErrorResponse::bad_request(format!("Malformed message cursor: {}", err)),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        // Upgrade the connection to WebSocket
        let (response, websocket) = match upgrade(request, None) {
            Ok(result) => result,
            Err(err) => {
                return msgpack(
                    ErrorResponse::internal_error(format!("WebSocket upgrade failed: {}", err)),
                    StatusCode::INTERNAL_SERVER_ERROR,
                );
            }
        };

        let serve_session = Arc::clone(&self.serve_session);

        // Spawn a task to handle the WebSocket connection
        tokio::spawn(async move {
            if let Err(e) = handle_websocket_subscription(
                serve_session,
                websocket,
                input_cursor,
                property_removals,
                tweeninfo_attributes,
                authority,
            )
            .await
            {
                log::error!("Error in websocket subscription: {}", e);
            }
        });

        response
    }

    async fn handle_api_write(&self, request: Request<Body>) -> Response<Body> {
        let body = match request_bytes(request, self.request_body_limit).await {
            Ok(body) => body,
            Err((err, status)) => return msgpack(err, status),
        };

        let request: WriteRequest = match deserialize_msgpack(&body) {
            Ok(request) => request,
            Err(err) => {
                return msgpack(
                    ErrorResponse::bad_request(format!("Invalid body: {}", err)),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        self.submit_write(request, self.serve_session.tree_mutation_sender())
    }

    fn submit_write(
        &self,
        request: WriteRequest,
        tree_mutation_sender: crossbeam_channel::Sender<PatchSet>,
    ) -> Response<Body> {
        let session_id = self.serve_session.session_id();
        if request.session_id != session_id {
            return msgpack(
                ErrorResponse::bad_request("Wrong session ID"),
                StatusCode::BAD_REQUEST,
            );
        }

        let updated_instances = request
            .updated
            .into_iter()
            .map(|update| PatchUpdate {
                id: update.id,
                changed_class_name: update.changed_class_name,
                changed_name: update.changed_name,
                changed_properties: update.changed_properties,
                changed_metadata: None,
            })
            .collect();

        if tree_mutation_sender
            .send(PatchSet {
                removed_instances: Vec::new(),
                added_instances: Vec::new(),
                updated_instances,
            })
            .is_err()
        {
            let details = format!(
                "/api/write could not submit changes for project {} because the change processor stopped. Check the serve terminal and restart rojo serve before reconnecting.",
                self.serve_session.root_project().file_location.display()
            );
            log::error!("{}", details);
            return msgpack(
                ErrorResponse::internal_error(details),
                StatusCode::SERVICE_UNAVAILABLE,
            );
        }

        msgpack_ok(WriteResponse { session_id })
    }

    async fn handle_api_read(&self, request: Request<Body>) -> Response<Body> {
        let authority = class_authority::negotiated(request.uri().query());
        let tweeninfo_attributes = tweeninfo_attributes::negotiated(request.uri().query());
        let show_paths = source_paths::negotiated(request.uri().query());
        let write_sources = request
            .uri()
            .query()
            .is_some_and(|query| query.split('&').any(|part| part == "writeReceiptsV1=true"));
        let argument = &request.uri().path()["/api/read/".len()..];
        let requested_ids: Result<Vec<Ref>, _> = argument.split(',').map(Ref::from_str).collect();

        let requested_ids = match requested_ids {
            Ok(ids) => ids,
            Err(_) => {
                return msgpack(
                    ErrorResponse::bad_request("Malformed ID list"),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        let message_queue = self.serve_session.message_queue();
        let message_cursor = message_queue.cursor();

        let tree = self.serve_session.tree();

        if !authority && class_authority::requires_client(&tree) {
            return msgpack(ErrorResponse::bad_request("Studio class authority requires a client with classAuthorityV1; update the plugin"), StatusCode::CONFLICT);
        }

        let mut instances = HashMap::new();

        for id in requested_ids {
            if let Some(instance) = tree.get_instance(id) {
                instances.insert(id, Instance::from_rojo_instance(instance));

                for descendant in tree.descendants(id) {
                    instances.insert(descendant.id(), Instance::from_rojo_instance(descendant));
                }
            }
        }

        for instance in instances.values_mut() {
            if let Err(error) = tweeninfo_attributes::adapt_instance(instance, tweeninfo_attributes)
            {
                return msgpack(
                    ErrorResponse::internal_error(format!(
                        "Could not encode legacy attributes: {error}"
                    )),
                    StatusCode::INTERNAL_SERVER_ERROR,
                );
            }
        }
        let response = ReadResponse {
            session_id: self.serve_session.session_id(),
            message_cursor,
            instances,
        };
        let bindings = write_sources.then(|| {
            crate::write_receipts::read_bindings(
                self.serve_session.session_id(),
                &tree,
                self.serve_session.vfs(),
                response.instances.keys().copied(),
            )
        });
        let authority = match authority
            .then(|| class_authority::read(&tree, response.instances.keys().copied()))
            .transpose()
        {
            Ok(authority) => authority,
            Err(error) => {
                return msgpack(
                    ErrorResponse::internal_error(format!(
                        "Could not resolve class authority: {error:#}"
                    )),
                    StatusCode::INTERNAL_SERVER_ERROR,
                )
            }
        };
        if show_paths {
            match source_paths::encode_read_with_authority(
                &tree,
                self.serve_session.vfs(),
                response,
                bindings,
                authority,
            ) {
                Ok(response) => response,
                Err(error) => msgpack(
                    ErrorResponse::internal_error(format!(
                        "Could not collect /api/read source paths: {error:#}"
                    )),
                    StatusCode::INTERNAL_SERVER_ERROR,
                ),
            }
        } else if bindings.is_some() || authority.is_some() {
            #[derive(serde::Serialize)]
            struct WithWriteSources<'a> {
                #[serde(flatten)]
                response: ReadResponse<'a>,
                #[serde(rename = "writeSources")]
                #[serde(skip_serializing_if = "Option::is_none")]
                bindings: Option<HashMap<Ref, crate::write_receipts::BindingView>>,
                #[serde(rename = "classAuthority", skip_serializing_if = "Option::is_none")]
                authority: Option<HashMap<Ref, &'static str>>,
            }
            msgpack_ok(WithWriteSources {
                response,
                bindings,
                authority,
            })
        } else {
            msgpack_ok(response)
        }
    }

    /// Accepts a list of IDs and returns them serialized as a binary model.
    /// The model is sent in a schema that causes Roblox to deserialize it as
    /// a Luau `buffer`.
    ///
    /// The returned model is a folder that contains ObjectValues with names
    /// that correspond to the requested Instances. These values have their
    /// `Value` property set to point to the requested Instance.
    async fn handle_api_serialize(&self, request: Request<Body>) -> Response<Body> {
        let isolated = request
            .uri()
            .query()
            .is_some_and(|query| query.split('&').any(|part| part == "isolateModels=true"));
        let session_id = self.serve_session.session_id();
        let body = match request_bytes(request, self.request_body_limit).await {
            Ok(body) => body,
            Err((err, status)) => return msgpack(err, status),
        };

        let request: SerializeRequest = match deserialize_msgpack(&body) {
            Ok(request) => request,
            Err(err) => {
                return msgpack(
                    ErrorResponse::bad_request(format!("Invalid body: {}", err)),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        if request.session_id != session_id {
            return msgpack(
                ErrorResponse::bad_request("Wrong session ID"),
                StatusCode::BAD_REQUEST,
            );
        }

        if isolated {
            return self.serialize_isolated(request.ids);
        }

        let mut response_dom = WeakDom::new(InstanceBuilder::new("Folder"));

        let tree = self.serve_session.tree();
        for id in &request.ids {
            if let Some(instance) = tree.get_instance(*id) {
                let clone = response_dom.insert(
                    Ref::none(),
                    InstanceBuilder::new(instance.class_name())
                        .with_name(instance.name())
                        .with_properties(instance.properties().clone()),
                );
                let object_value = response_dom.insert(
                    response_dom.root_ref(),
                    InstanceBuilder::new("ObjectValue")
                        .with_name(id.to_string())
                        .with_property("Value", clone),
                );

                let mut child_ref = clone;
                if let Some(parent_class) = parent_requirements(&instance.class_name()) {
                    child_ref =
                        response_dom.insert(object_value, InstanceBuilder::new(parent_class));
                    response_dom.transfer_within(clone, child_ref);
                }

                response_dom.transfer_within(child_ref, object_value);
            } else {
                return msgpack(
                    ErrorResponse::bad_request(format!("provided id {id} is not in the tree")),
                    StatusCode::BAD_REQUEST,
                );
            }
        }
        drop(tree);

        let mut source = Vec::new();
        if let Err(err) = crate::binary_serialization::to_writer(
            &mut source,
            &response_dom,
            &[response_dom.root_ref()],
        ) {
            let details = format!(
                "/api/serialize failed for project {}: {}",
                self.serve_session.root_project().file_location.display(),
                err
            );
            log::error!("{}", details);
            return msgpack(
                ErrorResponse::internal_error(details),
                StatusCode::INTERNAL_SERVER_ERROR,
            );
        }

        msgpack_ok(SerializeResponse {
            session_id: self.serve_session.session_id(),
            model_contents: source,
        })
    }

    fn serialize_isolated(&self, ids: Vec<Ref>) -> Response<Body> {
        #[derive(serde::Serialize)]
        #[serde(untagged)]
        enum Model {
            Success {
                id: Ref,
                #[serde(rename = "modelContents", with = "serde_bytes")]
                contents: Vec<u8>,
            },
            Failure {
                id: Ref,
                error: String,
            },
        }
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct IsolatedResponse {
            session_id: crate::session_id::SessionId,
            models: Vec<Model>,
            ref_patch: SubscribeMessage<'static>,
        }
        let mut seen = HashSet::new();
        let ids: Vec<_> = ids.into_iter().filter(|id| seen.insert(*id)).collect();
        let tree = self.serve_session.tree();
        let snapshots: Vec<_> = ids
            .iter()
            .map(|id| {
                let instance = tree.get_instance(*id)?;
                let mut dom = WeakDom::new(InstanceBuilder::new("Folder"));
                let clone = dom.insert(
                    Ref::none(),
                    InstanceBuilder::new(instance.class_name())
                        .with_name(instance.name())
                        .with_properties(instance.properties().clone()),
                );
                let wrapper = dom.insert(
                    dom.root_ref(),
                    InstanceBuilder::new("ObjectValue")
                        .with_name(id.to_string())
                        .with_property("Value", clone),
                );
                let parent = parent_requirements(&instance.class_name())
                    .map(|class| dom.insert(wrapper, InstanceBuilder::new(class)))
                    .unwrap_or(wrapper);
                dom.transfer_within(clone, parent);
                Some(dom)
            })
            .collect();
        // Both directions must come from the model snapshot: independent model
        // buffers cannot retain references to objects outside their own buffer.
        let mut updated = Vec::new();
        for instance in tree.descendants(tree.get_root_id()) {
            let properties: UstrMap<_> = instance
                .properties()
                .iter()
                .filter_map(|(name, value)| match value {
                    Variant::Ref(target)
                        if seen.contains(&instance.id()) || seen.contains(target) =>
                    {
                        Some((*name, Some(value.clone())))
                    }
                    _ => None,
                })
                .collect();
            if !properties.is_empty() {
                updated.push(InstanceUpdate {
                    id: instance.id(),
                    changed_name: None,
                    changed_class_name: None,
                    changed_metadata: None,
                    changed_properties: properties,
                });
            }
        }
        drop(tree);
        let models = ids
            .into_iter()
            .zip(snapshots)
            .map(|(id, dom)| {
                let Some(dom) = dom else {
                    return Model::Failure {
                        id,
                        error: format!("provided id {id} is not in the tree"),
                    };
                };
                let mut contents = Vec::new();
                match crate::binary_serialization::to_writer(&mut contents, &dom, &[dom.root_ref()])
                {
                    Ok(()) => Model::Success { id, contents },
                    Err(error) => Model::Failure {
                        id,
                        error: error.to_string(),
                    },
                }
            })
            .collect();
        msgpack_ok(IsolatedResponse {
            session_id: self.serve_session.session_id(),
            models,
            ref_patch: SubscribeMessage {
                added: HashMap::new(),
                removed: Vec::new(),
                updated,
            },
        })
    }

    /// Returns a list of all referent properties that point towards the
    /// provided IDs. Used because the plugin does not store a RojoTree,
    /// and referent properties need to be updated after the serialize
    /// endpoint is used.
    async fn handle_api_ref_patch(self, request: Request<Body>) -> Response<Body> {
        let session_id = self.serve_session.session_id();
        let body = match request_bytes(request, self.request_body_limit).await {
            Ok(body) => body,
            Err((err, status)) => return msgpack(err, status),
        };

        let request: RefPatchRequest = match deserialize_msgpack(&body) {
            Ok(request) => request,
            Err(err) => {
                return msgpack(
                    ErrorResponse::bad_request(format!("Invalid body: {}", err)),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        if request.session_id != session_id {
            return msgpack(
                ErrorResponse::bad_request("Wrong session ID"),
                StatusCode::BAD_REQUEST,
            );
        }

        let mut instance_updates: HashMap<Ref, InstanceUpdate> = HashMap::new();

        let tree = self.serve_session.tree();
        for instance in tree.descendants(tree.get_root_id()) {
            for (prop_name, prop_value) in instance.properties() {
                let Variant::Ref(prop_value) = prop_value else {
                    continue;
                };
                if let Some(target_id) = request.ids.get(prop_value) {
                    let instance_id = instance.id();
                    let update =
                        instance_updates
                            .entry(instance_id)
                            .or_insert_with(|| InstanceUpdate {
                                id: instance_id,
                                changed_class_name: None,
                                changed_name: None,
                                changed_metadata: None,
                                changed_properties: UstrMap::default(),
                            });
                    update
                        .changed_properties
                        .insert(*prop_name, Some(Variant::Ref(*target_id)));
                }
            }
        }

        msgpack_ok(RefPatchResponse {
            session_id: self.serve_session.session_id(),
            patch: SubscribeMessage {
                added: HashMap::new(),
                removed: Vec::new(),
                updated: instance_updates.into_values().collect(),
            },
        })
    }

    /// Open a script with the given ID in the user's default text editor.
    async fn handle_api_open(&self, request: Request<Body>) -> Response<Body> {
        // Opening a file launches a local program, so it must never be reachable
        // by a remote client even when the server is bound to an exposed address.
        //
        // `remote_addr` is the immediate peer, which is the best locality signal
        // we have: the legitimate caller is a sandboxed Roblox plugin whose only
        // credential is being able to reach the port, so there is no secret to
        // authenticate it with. A connection forwarded over loopback by an
        // SSH/Tailscale tunnel or a local reverse proxy therefore appears local
        // and is allowed. That is delegated trust rather than a bypass: by
        // standing up that tunnel or proxy the user has decided the remote end is
        // trusted, and reachability is bounded by that hop's own authentication
        // (e.g. SSH keys or Tailscale ACLs). This gate only stops direct,
        // unauthenticated peers.
        //
        // An IPv4 client reaching a dual-stack (`::`) bind appears as an
        // IPv4-mapped IPv6 peer (`::ffff:127.0.0.1`), so canonicalize to the bare
        // IPv4 form before the loopback test, matching `origin`'s handling.
        if !canonical(self.remote_addr.ip()).is_loopback() {
            return msgpack(
                ErrorResponse::forbidden("/api/open is only available to local clients"),
                StatusCode::FORBIDDEN,
            );
        }

        let argument = &request.uri().path()["/api/open/".len()..];
        let requested_id = match Ref::from_str(argument) {
            Ok(id) => id,
            Err(_) => {
                return msgpack(
                    ErrorResponse::bad_request("Invalid instance ID"),
                    StatusCode::BAD_REQUEST,
                );
            }
        };

        let tree = self.serve_session.tree();

        let instance = match tree.get_instance(requested_id) {
            Some(instance) => instance,
            None => {
                return msgpack(
                    ErrorResponse::bad_request("Instance not found"),
                    StatusCode::NOT_FOUND,
                );
            }
        };

        let script_path = match pick_script_path(instance) {
            Some(path) => path,
            None => {
                return msgpack(
                    ErrorResponse::bad_request(
                        "No appropriate file could be found to open this script",
                    ),
                    StatusCode::NOT_FOUND,
                );
            }
        };

        match opener::open(&script_path) {
            Ok(()) => {}
            Err(error) => match error {
                OpenError::Io(io_error) => {
                    return msgpack(
                        ErrorResponse::internal_error(format!(
                            "Attempting to open {} failed because of the following io error: {}",
                            script_path.display(),
                            io_error
                        )),
                        StatusCode::INTERNAL_SERVER_ERROR,
                    )
                }
                OpenError::ExitStatus {
                    cmd,
                    status,
                    stderr,
                } => {
                    return msgpack(
                        ErrorResponse::internal_error(format!(
                            r#"The command '{}' to open '{}' failed with the error code '{}'.
                            Error logs:
                            {}"#,
                            cmd,
                            script_path.display(),
                            status,
                            stderr
                        )),
                        StatusCode::INTERNAL_SERVER_ERROR,
                    )
                }
            },
        };

        msgpack_ok(OpenResponse {
            session_id: self.serve_session.session_id(),
        })
    }
}

/// If this instance is represented by a script, try to find the correct .lua or .luau
/// file to open to edit it.
fn pick_script_path(instance: InstanceWithMeta<'_>) -> Option<PathBuf> {
    match instance.class_name().as_str() {
        "Script" | "LocalScript" | "ModuleScript" => {}
        _ => return None,
    }

    // Pick the first listed relevant path that has an extension of .lua or .luau that
    // exists.
    instance
        .metadata()
        .relevant_paths
        .iter()
        .find(|path| {
            // We should only ever open Lua or Luau files to be safe.
            match path.extension().and_then(|ext| ext.to_str()) {
                Some("lua") => {}
                Some("luau") => {}
                _ => return false,
            }

            fs::metadata(path)
                .map(|meta| meta.is_file())
                .unwrap_or(false)
        })
        .map(|path| path.to_owned())
}

/// Handle WebSocket connection for streaming subscription messages
async fn handle_websocket_subscription(
    serve_session: Arc<ServeSession>,
    websocket: HyperWebsocket,
    input_cursor: u32,
    property_removals: bool,
    tweeninfo_attributes: bool,
    authority: bool,
) -> anyhow::Result<()> {
    let mut websocket = websocket.await?;

    let session_id = serve_session.session_id();
    let message_queue = serve_session.message_queue();

    log::debug!(
        "WebSocket subscription established for session {}",
        session_id
    );

    // Now continuously listen for new messages using select to handle both incoming messages
    // and WebSocket control messages concurrently
    let mut cursor = input_cursor;
    loop {
        let receiver = message_queue.subscribe(cursor);

        tokio::select! {
            // Handle new messages from the message queue
            result = receiver => {
                match result {
                    Ok((new_cursor, messages)) => {
                        if !messages.is_empty() {
                            anyhow::ensure!(authority || messages.iter().all(|patch|
                                patch.class_authority.values().all(|policy| matches!(policy, Ok(None | Some(crate::PropertyAuthority::Filesystem))))),
                                "Studio class authority requires classAuthorityV1; reconnect with an updated plugin");
                            let msgpack_message = {
                                let mut api_messages: Vec<_> = messages
                                    .iter()
                                    .map(SubscribeMessage::from_patch_update)
                                    .collect();

                                for message in &mut api_messages {
                                    tweeninfo_attributes::adapt_message(message, tweeninfo_attributes)?;
                                }

                                let response = SocketPacket {
                                    session_id,
                                    packet_type: SocketPacketType::Messages,
                                    body: SocketPacketBody::Messages(MessagesPacket {
                                        message_cursor: new_cursor,
                                        messages: api_messages,
                                    }),
                                };

                                property_removals::serialize_packet_with_authority(&response, property_removals, authority.then_some(messages.as_slice()))?
                            };

                            log::debug!("Sending batch of messages over WebSocket subscription");

                            if websocket.send(Message::Binary(msgpack_message)).await.is_err() {
                                // Client disconnected
                                log::debug!("WebSocket subscription closed by client");
                                break;
                            }
                            cursor = new_cursor;
                        }
                    }
                    Err(_) => {
                        // Message queue disconnected
                        log::debug!("Message queue disconnected; closing WebSocket subscription");
                        let _ = websocket.send(Message::Close(None)).await;
                        break;
                    }
                }
            }

            // Handle incoming WebSocket messages (ping/pong/close)
            msg = websocket.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) => {
                        log::debug!("WebSocket subscription closed by client");
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        // tungstenite handles pong automatically
                        log::debug!("Received ping: {:?}", data);
                    }
                    Some(Ok(Message::Pong(data))) => {
                        log::debug!("Received pong: {:?}", data);
                    }
                    Some(Ok(Message::Text(_))) | Some(Ok(Message::Binary(_))) => {
                        // Ignore text/binary messages from client for subscription endpoint
                        // TODO: Use this for bidirectional sync or requesting fallbacks?
                        log::debug!("Ignoring message from client since we don't use it for anything yet: {:?}", msg);
                    }
                    Some(Ok(Message::Frame(_))) => {
                        // This should never happen according to tungstenite docs
                        unreachable!();
                    }
                    Some(Err(e)) => {
                        log::error!("WebSocket error: {}", e);
                        break;
                    }
                    None => {
                        // WebSocket stream ended
                        log::debug!("WebSocket stream ended");
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Certain Instances MUST be a child of specific classes. This function
/// tracks that information for the Serialize endpoint.
///
/// If a parent requirement exists, it will be returned.
/// Otherwise returns `None`.
fn parent_requirements(class: &str) -> Option<&str> {
    Some(match class {
        "Attachment" | "Bone" => "Part",
        "Animator" => "Humanoid",
        "BaseWrap" | "WrapLayer" | "WrapTarget" | "WrapDeformer" => "MeshPart",
        _ => return None,
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::web::util::serialize_msgpack;
    use memofs::{InMemoryFs, Vfs, VfsSnapshot};

    #[tokio::test]
    async fn class_authority_reads_require_exact_negotiation_and_compose_with_paths_and_receipts() {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot("/project", VfsSnapshot::dir([("default.project.json", VfsSnapshot::file(r#"{"name":"Authority","syncAuthority":[{"className":"BasePart","properties":"studio"},{"className":"LuaSourceContainer","properties":"filesystem"}],"tree":{"$className":"Folder","Pole":{"$className":"Part"}}}"#))])).unwrap();
        let vfs = Vfs::new(fs);
        vfs.set_watch_enabled(false);
        let session = Arc::new(ServeSession::new(vfs, "/project/default.project.json").unwrap());
        let root = session.tree().get_root_id();
        let service = ApiService::new(session.clone(), "127.0.0.1:1234".parse().unwrap());
        for query in [
            "",
            "?classAuthorityV1=false",
            "?classAuthorityV1=True",
            "?classAuthorityV1=trueish",
            "?classAuthorityV1=true",
            "?showPaths=true&writeReceiptsV1=true&classAuthorityV1=true",
        ] {
            let response = service
                .handle_api_read(
                    Request::get(format!("/api/read/{root}{query}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            let expected = if query.ends_with("classAuthorityV1=true") {
                StatusCode::OK
            } else {
                StatusCode::CONFLICT
            };
            assert_eq!(response.status(), expected, "{query}");
            let bytes = body::to_bytes(response.into_body()).await.unwrap();
            let value: serde_json::Value = deserialize_msgpack(&bytes).unwrap();
            if expected == StatusCode::OK {
                let _: ReadResponse<'_> = deserialize_msgpack(&bytes).unwrap();
                let tree = session.tree();
                let part = tree.root().children()[0];
                assert_eq!(value["classAuthority"][root.to_string()], "legacy");
                assert_eq!(value["classAuthority"][part.to_string()], "studio");
                if query.contains("showPaths") {
                    assert!(value.get("writeSources").is_some());
                    assert!(value["instances"][root.to_string()]
                        .get("FilePaths")
                        .is_some());
                }
            } else {
                assert!(value["details"]
                    .as_str()
                    .unwrap()
                    .contains("classAuthorityV1"));
            }
        }
        let mut request = Request::get("/api/socket/0").body(Body::empty()).unwrap();
        assert_eq!(
            service.handle_api_socket(&mut request).await.status(),
            StatusCode::CONFLICT
        );
    }

    #[tokio::test]
    async fn read_source_paths_are_available_only_when_requested() {
        let session = session();
        let root = session.tree().get_root_id();
        let service = ApiService::new(session, "127.0.0.1:1234".parse().unwrap());
        for query in [
            "",
            "?showPaths=false",
            "?showPaths=TRUE",
            "?noshowPaths=true",
            "?showPaths=trueish",
            "?showPaths=true",
        ] {
            let response = service
                .handle_api_read(
                    Request::get(format!("/api/read/{root}{query}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await;
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = body::to_bytes(response.into_body()).await.unwrap();
            let legacy: ReadResponse = deserialize_msgpack(&bytes).unwrap();
            assert!(legacy.instances.contains_key(&root));
            let value: serde_json::Value = deserialize_msgpack(&bytes).unwrap();
            let paths = value["instances"][root.to_string()].get("FilePaths");
            if query == "?showPaths=true" {
                assert_eq!(
                    paths,
                    Some(&serde_json::json!(["/project/default.project.json"]))
                );
            } else {
                assert!(paths.is_none());
            }
        }
    }

    fn streamed_request(
        chunks: &[usize],
        declared_length: Option<&str>,
        abort: bool,
    ) -> Request<Body> {
        let (mut sender, body) = Body::channel();
        let chunks = chunks.to_vec();
        tokio::spawn(async move {
            for length in chunks {
                if sender.send_data(vec![0; length].into()).await.is_err() {
                    return;
                }
            }
            if abort {
                sender.abort();
            }
        });
        let mut request = Request::post("/api/write");
        if let Some(length) = declared_length {
            request = request.header(hyper::header::CONTENT_LENGTH, length);
        }
        request.body(body).unwrap()
    }

    #[tokio::test]
    async fn body_collector_enforces_streamed_and_declared_limits() {
        let limit = NonZeroUsize::new(16).unwrap();
        for (chunks, length, expected) in [
            (vec![8, 8], None, None),
            (vec![8, 8], Some("16"), None),
            (vec![8, 9], None, Some(StatusCode::PAYLOAD_TOO_LARGE)),
            (vec![8, 9], Some("1"), Some(StatusCode::PAYLOAD_TOO_LARGE)),
            (vec![], Some("17"), Some(StatusCode::PAYLOAD_TOO_LARGE)),
            (
                vec![],
                Some("18446744073709551615"),
                Some(StatusCode::PAYLOAD_TOO_LARGE),
            ),
        ] {
            let result = request_bytes(streamed_request(&chunks, length, false), limit).await;
            match expected {
                None => assert_eq!(result.unwrap().len(), 16),
                Some(expected) => {
                    let (error, status) = result.unwrap_err();
                    assert_eq!(status, expected);
                    let error = serde_json::to_value(error).unwrap();
                    let details = error["details"].as_str().unwrap();
                    assert!(details.contains("/api/write") && details.contains("16"));
                    assert!(details.contains("--max-request-body-bytes"));
                }
            }
        }
        let (_, status) = request_bytes(streamed_request(&[8], None, true), limit)
            .await
            .unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn all_body_routes_return_structured_limit_errors_and_remain_healthy() {
        let session = session();
        let root_name = session.tree().root().name().to_owned();
        let limit = NonZeroUsize::new(16).unwrap();
        for route in ["/api/write", "/api/serialize", "/api/ref-patch"] {
            let mut request = streamed_request(&[8, 9], None, false);
            *request.uri_mut() = route.parse().unwrap();
            let response = call_with_body_limit(
                session.clone(),
                "127.0.0.1:1234".parse().unwrap(),
                request,
                limit,
            )
            .await;
            assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
            assert_eq!(
                response.headers()[hyper::header::CONTENT_TYPE],
                "application/msgpack"
            );
            let bytes = body::to_bytes(response.into_body()).await.unwrap();
            let error: serde_json::Value = deserialize_msgpack(&bytes).unwrap();
            assert_eq!(error["kind"], "BadRequest");
            assert!(error["details"].as_str().unwrap().contains(route));
            assert_eq!(session.tree().root().name(), root_name);
            let response = call_with_body_limit(
                session.clone(),
                "127.0.0.1:1234".parse().unwrap(),
                Request::get("/api/rojo").body(Body::empty()).unwrap(),
                limit,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn configured_limit_accepts_a_valid_request_at_its_exact_size() {
        let session = session();
        let payload = serialize_msgpack(WriteRequest {
            session_id: session.session_id(),
            removed: Vec::new(),
            added: HashMap::new(),
            updated: Vec::new(),
        })
        .unwrap();
        for (limit, expected) in [
            (payload.len() - 1, StatusCode::PAYLOAD_TOO_LARGE),
            (payload.len(), StatusCode::OK),
        ] {
            let response = call_with_body_limit(
                session.clone(),
                "127.0.0.1:1234".parse().unwrap(),
                Request::post("/api/write")
                    .body(Body::from(payload.clone()))
                    .unwrap(),
                NonZeroUsize::new(limit).unwrap(),
            )
            .await;
            assert_eq!(response.status(), expected);
        }
    }

    #[tokio::test]
    async fn oversized_http_body_is_rejected_before_parsing() {
        for route in ["/api/write", "/api/serialize", "/api/ref-patch"] {
            let request = Request::post(route)
                .body(Body::from(vec![0; 64 * 1024 * 1024 + 1]))
                .unwrap();
            let response = call(session(), "127.0.0.1:1234".parse().unwrap(), request).await;
            assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE, "{route}");
        }
    }

    fn session() -> Arc<ServeSession> {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot(
            "/project",
            VfsSnapshot::dir([(
                "default.project.json",
                VfsSnapshot::file(r#"{"name":"API","tree":{"$className":"Folder"}}"#),
            )]),
        )
        .unwrap();
        let vfs = Vfs::new(fs);
        vfs.set_watch_enabled(false);
        Arc::new(ServeSession::new(vfs, "/project/default.project.json").unwrap())
    }

    #[tokio::test]
    async fn property_removal_capability_preserves_legacy_info_deserialization() {
        let session = session();
        let service = ApiService::new(session.clone(), "127.0.0.1:1234".parse().unwrap());
        let response = service.handle_api_rojo().await;
        let bytes = body::to_bytes(response.into_body()).await.unwrap();
        let info: ServerInfoResponse = deserialize_msgpack(&bytes).unwrap();
        assert_eq!(info.session_id, session.session_id());
        assert_eq!(info.protocol_version, PROTOCOL_VERSION);
        let extended: serde_json::Value = deserialize_msgpack(&bytes).unwrap();
        assert_eq!(
            extended["capabilities"],
            serde_json::json!([
                property_removals::CAPABILITY,
                tweeninfo_attributes::CAPABILITY,
                "serializedModelsV1",
                crate::write_receipts::CAPABILITY,
                class_authority::CAPABILITY
            ])
        );
    }

    #[tokio::test]
    async fn debugger_routes_and_capability_require_server_opt_in() {
        let session = session();
        for enabled in [false, true] {
            let broker = enabled.then(Default::default);
            let response = call_with_options(
                session.clone(),
                "127.0.0.1:1234".parse().unwrap(),
                Request::get("/api/rojo").body(Body::empty()).unwrap(),
                NonZeroUsize::new(1024).unwrap(),
                [false; 4],
                broker.clone(),
            )
            .await;
            let bytes = body::to_bytes(response.into_body()).await.unwrap();
            let legacy: ServerInfoResponse = deserialize_msgpack(&bytes).unwrap();
            assert_eq!(legacy.protocol_version, 5);
            let info: serde_json::Value = deserialize_msgpack(&bytes).unwrap();
            assert_eq!(
                info["capabilities"]
                    .as_array()
                    .unwrap()
                    .contains(&serde_json::json!("debuggerPlansV1")),
                enabled
            );
            assert_eq!(info.get("debuggerPlans").is_some(), enabled);
            if enabled {
                assert_eq!(info["debuggerPlans"]["heldPause"], false);
                assert_eq!(info["debuggerPlans"]["separatePlay"], false);
            }
            let response = call_with_options(
                session.clone(),
                "127.0.0.1:1234".parse().unwrap(),
                Request::get(format!(
                    "/api/debugger/status?sessionId={}",
                    session.session_id()
                ))
                .body(Body::empty())
                .unwrap(),
                NonZeroUsize::new(1024).unwrap(),
                [false; 4],
                broker,
            )
            .await;
            assert_eq!(
                response.status(),
                if enabled {
                    StatusCode::OK
                } else {
                    StatusCode::NOT_FOUND
                }
            );
        }
    }

    #[tokio::test]
    async fn tweeninfo_reads_require_opt_in_for_new_attribute_encoding() {
        let mut fs = InMemoryFs::new();
        fs.load_snapshot("/theme", VfsSnapshot::dir([
            ("default.project.json", VfsSnapshot::file(r#"{"name":"Theme","tree":{"$path":"theme.rbxmx"}}"#)),
            ("theme.rbxmx", VfsSnapshot::file(r#"<roblox version="4"><Item class="StyleSheet" referent="R"><Properties><BinaryString name="AttributesSerialize">AgAAAAsAAABBY2NlbnRDb2xvcgIFAAAAJFBpbmsLAAAATW90aW9uSG92ZXIkCtcjPAAAAAAAAAAAAwAAAAAAAAAA</BinaryString></Properties></Item></roblox>"#)),
        ])).unwrap();
        let vfs = Vfs::new(fs);
        vfs.set_watch_enabled(false);
        let session = Arc::new(ServeSession::new(vfs, "/theme/default.project.json").unwrap());
        let root = session.tree().get_root_id();
        let service = ApiService::new(session, "127.0.0.1:1234".parse().unwrap());
        for (query, encoding) in [
            ("", "BinaryString"),
            ("?attributeTweenInfoV1=true", "Attributes"),
            ("?showPaths=true&attributeTweenInfoV1=true", "Attributes"),
        ] {
            let request = Request::get(format!("/api/read/{root}{query}"))
                .body(Body::empty())
                .unwrap();
            let response = service.handle_api_read(request).await;
            let bytes = body::to_bytes(response.into_body()).await.unwrap();
            let decoded: serde_json::Value = deserialize_msgpack(&bytes).unwrap();
            assert!(
                decoded["instances"][root.to_string()]["Properties"]["Attributes"]
                    .get(encoding)
                    .is_some(),
                "{query}: {decoded}"
            );
        }
    }

    async fn aborted_body_returns_bad_request(path: &str) {
        let session = session();
        let (sender, body) = Body::channel();
        sender.abort();
        let request = Request::post(path).body(body).unwrap();
        let response = call(session.clone(), "127.0.0.1:1234".parse().unwrap(), request).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.headers()[hyper::header::CONTENT_TYPE],
            "application/msgpack"
        );
        let bytes = body::to_bytes(response.into_body()).await.unwrap();
        let error: serde_json::Value = deserialize_msgpack(&bytes).unwrap();
        assert_eq!(error["kind"], "BadRequest");
        assert!(error["details"].as_str().unwrap().contains(path));
        let response = call(
            session,
            "127.0.0.1:1234".parse().unwrap(),
            Request::get("/api/rojo").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn aborted_write_body_returns_bad_request() {
        aborted_body_returns_bad_request("/api/write").await;
    }

    #[tokio::test]
    async fn aborted_serialize_body_returns_bad_request() {
        aborted_body_returns_bad_request("/api/serialize").await;
    }

    #[tokio::test]
    async fn aborted_ref_patch_body_returns_bad_request() {
        aborted_body_returns_bad_request("/api/ref-patch").await;
    }

    #[tokio::test]
    async fn stopped_change_processor_rejects_write_with_context() {
        let session = session();
        let service = ApiService::new(session.clone(), "127.0.0.1:1234".parse().unwrap());
        let (sender, receiver) = crossbeam_channel::unbounded();
        drop(receiver);
        let response = service.submit_write(
            WriteRequest {
                session_id: session.session_id(),
                updated: Vec::new(),
                added: HashMap::new(),
                removed: Vec::new(),
            },
            sender,
        );
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let bytes = body::to_bytes(response.into_body()).await.unwrap();
        let error: serde_json::Value = deserialize_msgpack(&bytes).unwrap();
        assert_eq!(error["kind"], "InternalError");
        let details = error["details"].as_str().unwrap();
        for context in ["/api/write", "default.project.json", "restart"] {
            assert!(details.contains(context), "{details}");
        }
        assert_eq!(session.message_queue().cursor(), 0);
        let response = service.handle_api_rojo().await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn isolated_models_keep_healthy_entries_and_complete_references() {
        use crate::snapshot::InstanceSnapshot;
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Entry {
            id: Ref,
            #[serde(default, with = "serde_bytes")]
            model_contents: Vec<u8>,
            error: Option<String>,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ResponseBody {
            models: Vec<Entry>,
            ref_patch: SubscribeMessage<'static>,
        }
        let session = session();
        let (good, bad, outside) = {
            let mut tree = session.tree();
            let root = tree.get_root_id();
            let good = tree.insert_instance(
                root,
                InstanceSnapshot::new()
                    .name("Good")
                    .class_name("ObjectValue"),
            );
            let bad = tree.insert_instance(
                root,
                InstanceSnapshot::new()
                    .name("Bad")
                    .class_name("Part")
                    .property("Archivable", "invalid boolean"),
            );
            let outside = tree.insert_instance(
                root,
                InstanceSnapshot::new()
                    .name("Outside")
                    .class_name("ObjectValue")
                    .property("Value", good),
            );
            tree.get_instance_mut(good)
                .unwrap()
                .properties_mut()
                .insert("Value".into(), Variant::Ref(outside));
            tree.get_instance_mut(root)
                .unwrap()
                .properties_mut()
                .insert("NilRef".into(), Variant::Ref(Ref::none()));
            (good, bad, outside)
        };
        let root = session.tree().get_root_id();
        let missing = Ref::new();
        let request = |query: &str, ids: Vec<Ref>| {
            Request::post(format!("/api/serialize{query}"))
                .body(Body::from(
                    serialize_msgpack(SerializeRequest {
                        session_id: session.session_id(),
                        ids,
                    })
                    .unwrap(),
                ))
                .unwrap()
        };
        let response = call(
            session.clone(),
            "127.0.0.1:1234".parse().unwrap(),
            request("?isolateModels=true", vec![good, bad, missing, good, root]),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = body::to_bytes(response.into_body()).await.unwrap();
        let response: ResponseBody = deserialize_msgpack(&bytes).unwrap();
        assert_eq!(
            response
                .models
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            vec![good, bad, missing, session.tree().get_root_id()]
        );
        assert!(response.models[0].error.is_none());
        let dom = rbx_binary::from_reader(response.models[0].model_contents.as_slice()).unwrap();
        assert!(dom.descendants().any(|instance| instance.name == "Good"));
        assert!(response.models[1]
            .error
            .as_ref()
            .unwrap()
            .contains("Archivable"));
        assert!(response.models[2].error.is_some());
        for (id, name, target) in [
            (good, "Value", outside),
            (outside, "Value", good),
            (session.tree().get_root_id(), "NilRef", Ref::none()),
        ] {
            let update = response
                .ref_patch
                .updated
                .iter()
                .find(|update| update.id == id)
                .unwrap();
            assert_eq!(
                update.changed_properties.get(&name.into()),
                Some(&Some(Variant::Ref(target)))
            );
        }
        for query in [
            "",
            "?isolateModels=false",
            "?isolateModels=TRUE",
            "?isolateModels=trueish",
        ] {
            let response = call(
                session.clone(),
                "127.0.0.1:1234".parse().unwrap(),
                request(query, vec![good]),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = body::to_bytes(response.into_body()).await.unwrap();
            let legacy: SerializeResponse = deserialize_msgpack(&bytes).unwrap();
            assert!(!legacy.model_contents.is_empty());
        }
    }

    #[tokio::test]
    async fn serialization_failure_returns_context_and_keeps_session_usable() {
        let session = session();
        let id = {
            let mut tree = session.tree();
            let id = tree.get_root_id();
            tree.get_instance_mut(id).unwrap().properties_mut().insert(
                "Archivable".into(),
                Variant::String("invalid boolean".into()),
            );
            id
        };
        let request = Request::post("/api/serialize")
            .body(Body::from(
                serialize_msgpack(SerializeRequest {
                    session_id: session.session_id(),
                    ids: vec![id],
                })
                .unwrap(),
            ))
            .unwrap();
        let response = call(session.clone(), "127.0.0.1:1234".parse().unwrap(), request).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = body::to_bytes(response.into_body()).await.unwrap();
        let error: serde_json::Value = deserialize_msgpack(&bytes).unwrap();
        assert_eq!(error["kind"], "InternalError");
        let details = error["details"].as_str().unwrap();
        assert!(details.contains("/api/serialize"), "{details}");
        assert!(details.contains("default.project.json"), "{details}");
        assert!(details.contains("Archivable"), "{details}");
        let response = call(
            session,
            "127.0.0.1:1234".parse().unwrap(),
            Request::get("/api/rojo").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}
