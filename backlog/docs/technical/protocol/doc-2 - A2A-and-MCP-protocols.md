---
id: doc-2
title: A2A and MCP protocols
type: technical
audience: technical
created_date: "2026-09-29 23:40"
---

# A2A and MCP protocols

## A2A

The server speaks Agent2Agent (A2A) 1.0 over HTTP+JSON, JSON-RPC, and gRPC.

It uses the official `a2a-lf` models, the `a2a-server-lf` request handler, and `a2a-grpc`.

This crate depends on `a2a-lf` 0.4.1, `a2a-server-lf` 0.5.1, `a2a-client-lf` 0.2.7, and `a2a-grpc` 0.3.

### Card and routes

The server publishes an Agent Card at `/.well-known/agent-card.json`.

Protocol version `1.0` is on the `HTTP+JSON`, `JSONRPC`, and `GRPC` interfaces.

JSON-RPC is `POST /` on the HTTP listener.

gRPC listens on a second socket.

The card sets the `GRPC` interface URL to `http://` plus that socket.

The card advertises `streaming`.

It advertises `pushNotifications` and `extendedAgentCard` only when those features are configured on `A2aServer`.

The seventeen lab skills are:

- `list-log-sources`
- `query-logs`
- `list-metrics`
- `query-metric`
- `list-tasks`
- `start-task`
- `get-task-status`
- `list-image-sources`
- `list-images`
- `search-images`
- `get-image`
- `get-current-image`
- `list-video-sources`
- `get-video-frame-at`
- `get-video-clip`
- `get-video-timeline`
- `follow-video`

Clients send a `message` to `POST /message:send` or `POST /message:stream`.

Requests use the header `A2A-Version: 1.0`.

Requests use `application/a2a+json` or `application/json`. A successful response uses `application/json`.

The lab command is a data part with media type `application/json`:

```json
{
  "operation": "query_logs",
  "params": {}
}
```

A successful send response is a ProtoJSON task envelope.

Failures are `google.rpc.Status` bodies with `ErrorInfo`.

The artifact data part uses the same media type:

```json
{
  "operation": "query_logs",
  "result": {}
}
```

### Agent messages

`A2aServer::with_message_handler` adds the `agent-message` skill.

A `ROLE_USER` message whose parts are all `text/plain` goes to that handler.

The handler receives the server `contextId`, the protocol task id, and any `referenceTaskIds`.

The handler text comes back as a `text/plain` artifact.

The protocol task completes.

The next turn reuses `contextId` and gets a new task.

A completed task does not accept another message.

The client keeps the sender-generated message id.

A lab data part bypasses the handler.

With a handler, any other part is rejected.

A client role other than `ROLE_USER` is rejected.

With no handler, a message without a lab command completes with the profile help artifact.

The card then keeps the seventeen lab skills.

### Security

`with_security` applies its requirements to protocol calls.

Requirements in the list are alternatives.

Schemes inside one requirement must all succeed.

HTTP bearer, HTTP basic, and API key credentials identify a caller by a fingerprint of the secret when no authenticator is configured.

OAuth2 and OpenID Connect require `with_authenticator`.

`OidcAuthenticator` checks RS256 access tokens against the issuer discovery document and JWKS.

The checks are signature, issuer, audience, expiry, not-before, and the scopes named by the requirement.

The caller stored with a task is the token `sub`.

Missing or invalid credentials are HTTP 401 with `WWW-Authenticate` and gRPC `UNAUTHENTICATED`.

A valid token without the required scopes is HTTP 403 and gRPC `PERMISSION_DENIED`.

Mutual TLS fails closed.

Tasks and contexts created by one caller are hidden from the others.

The public Agent Card stays unauthenticated.

Clients obtain a token out of band, including the OAuth2 client-credentials grant, and send it with `A2aClient::with_bearer_token`.

Outside an isolated test network, publish the Agent Card over TLS.

### Tasks and streams

Task states use the protocol names `TASK_STATE_SUBMITTED`, `TASK_STATE_WORKING`, `TASK_STATE_COMPLETED`, `TASK_STATE_FAILED`, and `TASK_STATE_CANCELED`.

A custom executor can also produce `INPUT_REQUIRED`, `AUTH_REQUIRED`, and `REJECTED`.

A lab run id is separate from the A2A task id.

`GET /tasks/{id}` loads the protocol task.

Lab run status stays on the `get-task-status` skill.

For `start_task`, the executor sets `wait` to false and polls until the run is terminal.

`POST /tasks/{id}:subscribe` and `POST /message:stream` emit live `StreamResponse` frames.

Frame kinds are `task`, `statusUpdate`, `artifactUpdate`, and `message`.

Query-log, query-metric, list-image-sources, list-images, and search-images pages become one artifact chunk per item.

Later chunks set `append`.

The last chunk sets `lastChunk` to true.

A provider that cannot cancel a run returns `TASK_NOT_CANCELABLE`.

`invalid` becomes `invalid_params`.

`not_found` becomes `task_not_found`.

`unavailable` and `transport` become `internal`, except a non-cancelable run.

A protocol media-type failure becomes `CONTENT_TYPE_NOT_SUPPORTED`.

Other `protocol` failures become `invalid_request`.

`TASK_NOT_CANCELABLE` maps to HTTP 409.

`CONTENT_TYPE_NOT_SUPPORTED` maps to HTTP 415.

Authentication failures are HTTP 401 or 403 before protocol dispatch.

Other JSON-RPC errors stay HTTP 200 with the code in the envelope.

### Images

Image fields, current-frame meaning, and payload sizing are in [A2A-LAB primitives](<../../reference/primitives/doc-17 - A2A-LAB-primitives.md>).

List and search skills return descriptors. They do not return pixel bytes. `get-image` and `get-current-image` return one JSON data part. The `data` field is standard base64. Those two skills are not split into per-item chunks, and they do not return a URI.

`A2aClient::with_image_transport` applies a raised decoded-byte limit while the image JSON is decoded. `Image` deserialization stays at 64 MiB unless that config is passed. Set the same maximum on the service with `A2aLabService::with_image_transport`.

This crate does not add a source helper, a capture helper, or a built-in HTTP, USB, or IP-camera adapter. SiLA image retrieval is the separate `LabImages` feature.

### Videos

Video fields, timeline intervals, playback sessions, frame-at-time extraction, live follow, and the four availability codes are in [A2A-LAB primitives](<../../reference/primitives/doc-17 - A2A-LAB-primitives.md>). Saved still images remain exclusively in the Images primitive. Videos neither browses nor owns them. `get-video-frame-at` and `get_video_frame_at` remain the public names. `get-video-clip` and `get_video_clip` remain the public names. The Videos page does not expose that extraction.

`list-video-sources` returns a page of sources. `get-video-timeline` returns ordered retained intervals, an optional newest timestamp, and whether live follow can be prepared. Empty retention succeeds with no intervals and no newest timestamp. `get-video-frame-at` returns one extracted JPEG from retained video as image JSON. That payload is not a saved image. `get-video-clip` returns finite-session metadata for one half-open retained range: id, source, requested and effective ranges, media type, a relative gateway root `/media/video-clips/{id}/`, and optional 8x trick-play metadata. `follow-video` returns a live session that is metadata only: an opaque id, the source id, media type, a relative gateway root `/media/video-live/{id}/`, and optional 8x metadata. Neither JSON body includes video bytes, playlists, or credentials. There is no 60-second catalog limit.

`GET /media/video-clips/{id}/` and `GET /media/video-live/{id}/` are the gateway roots. Relative assets under those roots include bounded HLS playlists (`index.m3u8`, optional `index-8x.m3u8`), `video/mp4` init, `.m4s` segments, or a finite MJPEG multipart body for MJPEG sources. The player never assigns an authenticated HLS URL to `video.src`. The bearer token is sent only in the `Authorization` header. Query strings do not carry it. Every gateway response uses `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`. There is no public route, byte range, or shared cache. When security requirements are configured, those `/media/video-*` routes use the same access gate as protocol routes on the A2A listener and on the MCP HTTP listener. They are not public `/ui` routes.

Availability failures keep stable codes: `no_recording`, `recording_gap`, `clip_expired`, and `live_unavailable`. A2A carries them in `ErrorInfo` `metadata.code` on an `INTERNAL_ERROR`. Generic `invalid`, `not_found`, and `unavailable` stay those codes. They do not always use dedicated sentences on the A2A task path. The web interface shows "The video result could not be read." for invalid and not_found task responses, and the provider text for a generic unavailable response. Availability codes do travel as metadata codes, and the page uses the four sentences when it reads those codes. [Open the optional web interface](<../../guide/web-interface/doc-22 - Open-the-optional-web-interface.md>) states those sentences.

This crate supplies contracts and transport. It does not capture cameras and does not persist recordings. SiLA video retrieval is the separate `LabVideos` feature. `LabVideos` 1.1 has no follow command.

### Conformance

`tests/a2a_compliance.rs` checks the HTTP+JSON wire contract, JSON-RPC envelopes, gRPC calls, SSE frames, and the `tck-*` message-id profiles.

`mise run tck` runs the pinned official suite in a container against `examples/a2a_interface`.

The suite covers HTTP+JSON, JSON-RPC, and gRPC.

It runs MUST, SHOULD, and MAY tests, including advertised streaming.

MUST failures fail the run.

Optional push and extended-card declarations stay out of that core set until they are configured.

The card advertises an optional capability only when that capability is implemented and configured.

The pinned TCK registers `AUTH-*` requirements but does not execute them.

It does not inject `A2A_AUTH_*` credentials.

Those requirements stay upstream NOT TESTED.

`mise run test` starts an ephemeral pre-seeded Keycloak container for the OIDC checks in `tests/a2a_authentication.rs`.

That file also covers issuer, audience, expiry, signature, and scope failures with a local fixture.

`examples/a2a_interface` stays anonymous unless `A2A_TCK_OIDC_ISSUER` is set.

`HTTP_JSON-SVC-001` checks that a successful response uses `Content-Type: application/json`.

`CORE-SEND-003` sends a part whose media type is `application/x-unsupported-tck-type` and expects the send to succeed.

## MCP

`McpServer` targets Model Context Protocol (MCP) `2026-07-28` through `rmcp` 3.5.

The tools are `list_log_sources`, `query_logs`, `list_metrics`, `query_metric`, `list_tasks`, `start_task`, `get_task_status`, `list_image_sources`, `list_images`, `search_images`, `get_image`, `get_current_image`, `list_video_sources`, `get_video_frame_at`, `get_video_clip`, `get_video_timeline`, and `follow_video`.

The server enables tools only. It does not expose MCP resources, URI-only image delivery, or capture tools.

`list_image_sources`, `list_images`, and `search_images` return metadata pages in the `application/json` tool result. `get_image` and `get_current_image` return the descriptor plus standard base64 in that same envelope. The base64 payload is repeated in the text content and the structured content. The Streamable HTTP event budget follows the decoded-byte limit and allows for that repetition.

`get_current_image` names a source. Current-frame meaning and payload sizing are in [A2A-LAB primitives](<../../reference/primitives/doc-17 - A2A-LAB-primitives.md>).

`McpLab` defaults to 64 MiB of decoded bytes. `McpLab::connect_with` must receive a raised limit before the connection opens so the HTTP event window grows. `McpLab::with_image_transport` after connect changes decoding only.

`list_video_sources` returns a metadata page. `get_video_timeline` returns ordered retained intervals. `get_video_frame_at` returns one extracted frame as image JSON. The public name stays `get_video_frame_at`. Saved still images remain exclusively in the Images primitive. Videos neither browses nor owns them. `get_video_clip` returns clip metadata and a relative `/media/video-clips/{id}/` gateway root. `follow_video` returns a live session that is metadata only: an opaque id, the source id, media type, a relative `/media/video-live/{id}/` root, and optional 8x metadata. Tool JSON does not include video bytes.

`McpServer::serve_http` mounts `GET /media/video-clips/{id}/` and `GET /media/video-live/{id}/` (and nested relative assets) beside `/mcp`. When `McpServer::with_security` has non-empty requirements, those `/media/video-*` routes use the access gate. The `/mcp` host allowlist still does not cover them. Responses are `no-store` and `nosniff`. The media router adds no byte-range response and no shared cache. A bearer token belongs only in the `Authorization` header. `McpLab` fetches a clip from the MCP origin; it does not put the token in a query.

Tool errors put `A2aLabError::code` in `data.code`. `no_recording`, `recording_gap`, and `clip_expired` are invalid-params errors. `live_unavailable` is an internal error, as are `unavailable` and `transport`. Media `GET` failures are JSON `{"error":{"code","message"}}`, and `error.code` is the same stable code.

This MCP surface does not add a source helper, a capture helper, or a built-in source adapter. SiLA image retrieval is the separate `LabImages` feature. SiLA video retrieval is the separate `LabVideos` feature, which has no follow command. The kit supplies contracts and transport, not camera capture or persistence.

Input and output schemas come from the same request and result types used by A2A.

`serve_stdio` uses newline-delimited JSON-RPC on standard input and output.

`serve_http` mounts Streamable HTTP at `/mcp`.

Accepted `Host` values for `/mcp` are `localhost`, `127.0.0.1`, `::1`, and `host.docker.internal`. A listener on every interface accepts any `Host` for `/mcp`. That allowlist does not cover `/media/*`.

With no listener, the MCP server binds `127.0.0.1:31001`.

With no listener, `A2aServer::listen` binds `127.0.0.1:31000`.

`McpLab::connect_default` calls `http://127.0.0.1:31001/mcp`.

`start_task` waits until the run is terminal unless `wait` is false.

`timeout_seconds` defaults to 60.

Set `wait` to false to return when the run is accepted.

Poll that run with `get_task_status`.

`invalid`, `protocol`, and `not_found` become MCP invalid-params errors.

`unavailable` and `transport` become internal errors.

`no_recording`, `recording_gap`, and `clip_expired` become invalid-params errors. `live_unavailable` becomes an internal error.

Error data includes the `A2aLabError` code in `data.code`. Media `GET` uses `error.code`.

The architecture is described in [A2A-LAB devkit architecture](<../architecture/doc-1 - Lab-dev-kit-architecture.md>).
