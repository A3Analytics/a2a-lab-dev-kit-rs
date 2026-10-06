---
id: doc-2
title: A2A and MCP protocols
type: technical
audience: technical
created_date: "2026-09-29 23:40"
---

# A2A and MCP protocols

## A2A

The server speaks A2A 1.0 over HTTP+JSON, JSON-RPC, and gRPC using the official `a2a-lf` models, `a2a-server-lf` request handler, and `a2a-grpc`. It publishes an Agent Card at `/.well-known/agent-card.json` with seven skills:

- `list-log-sources`
- `query-logs`
- `list-metrics`
- `query-metric`
- `list-tasks`
- `start-task`
- `get-task-status`

Protocol version `1.0` is on the `HTTP+JSON`, `JSONRPC`, and `GRPC` interfaces. JSON-RPC is `POST /` on the HTTP listener. gRPC listens on a second socket, and the card publishes that address as `host:port`. The card advertises `streaming`. It advertises `pushNotifications` and `extendedAgentCard` only when those features are configured on `A2aServer`.

Clients send a `message` to `POST /message:send` or `POST /message:stream`. The HTTP body is `application/a2a+json`. The lab command is a data part with media type `application/json`:

```json
{
  "operation": "query_logs",
  "params": {}
}
```

`A2aServer::with_message_handler` adds an eighth skill, `agent-message`. A `ROLE_USER` message whose parts are all `text/plain` is passed to that handler with the server `contextId`, the protocol task id, and any `referenceTaskIds`. The handler's text comes back as a `text/plain` artifact and the protocol task completes. The next turn reuses `contextId` and gets a new task, because a completed task does not accept another message. The client keeps the sender-generated message id. A lab data part still bypasses the handler. With a handler, any other part is rejected. A client role other than `ROLE_USER` is rejected. With no handler configured, a message without a lab command completes with the profile help artifact and the card keeps the seven lab skills.

`with_security` applies its requirements to protocol calls. Requirements in the list are alternatives. Schemes inside one requirement must all succeed. HTTP bearer, HTTP basic, and API key credentials identify a caller by a fingerprint of the secret when no authenticator is configured. OAuth2 and OpenID Connect require `with_authenticator`. `OidcAuthenticator` checks RS256 access tokens against the issuer's discovery document and JWKS: signature, issuer, audience, expiry, not-before, and the scopes named by the requirement. The caller stored with a task is the token `sub`, not the bearer token. Missing or invalid credentials are HTTP 401 with `WWW-Authenticate` and gRPC `UNAUTHENTICATED`. A valid token without the required scopes is HTTP 403 and gRPC `PERMISSION_DENIED`. Mutual TLS fails closed. Tasks and contexts created by one caller are hidden from the others. The public Agent Card stays unauthenticated. Clients obtain a token out of band, including the OAuth2 client-credentials grant, and send it with `A2aClient::with_bearer_token`. Outside an isolated test network, publish the Agent Card over TLS.

A successful send response is a ProtoJSON task envelope. Its artifact data part uses the same media type:

```json
{
  "operation": "query_logs",
  "result": {}
}
```

Task states use the protocol names `TASK_STATE_SUBMITTED`, `TASK_STATE_WORKING`, `TASK_STATE_COMPLETED`, `TASK_STATE_FAILED`, `TASK_STATE_CANCELED`, and the extra official states `INPUT_REQUIRED`, `AUTH_REQUIRED`, and `REJECTED` when a custom executor produces them. A lab `start_task` run id is not the A2A task id. `GET /tasks/{id}` loads the protocol task. Lab run status stays on the `get_task_status` skill.

`POST /tasks/{id}:subscribe` and `POST /message:stream` emit live `StreamResponse` frames. Query results are split into ordered artifact chunks. The last chunk sets `lastChunk` to true. Subscribe after a terminal task returns `unsupported_operation`.

Core conformance is `tests/a2a_compliance.rs` plus `mise run tck`, which runs the official MUST, SHOULD, and MAY suite in a container against `examples/a2a_tck` for HTTP+JSON, JSON-RPC, and gRPC, including advertised streaming. MUST failures fail the run. Optional push and extended-card declarations are out of that core set until they are configured. The pinned TCK registers `AUTH-*` requirements but does not execute them and does not inject `A2A_AUTH_*` credentials, so those requirements stay upstream NOT TESTED. `mise run test` starts an ephemeral pre-seeded Keycloak container for the OIDC checks in `tests/a2a_authentication.rs`. That file also covers issuer, audience, expiry, signature, and scope failures with a local fixture. `examples/a2a_tck` stays anonymous unless `A2A_TCK_OIDC_ISSUER` is set. The TCK pin still expects success `Content-Type: application/json`; this crate emits `application/a2a+json` and deselects that one content-type assertion. A part `mediaType` other than `text/plain` or `application/json` returns `CONTENT_TYPE_NOT_SUPPORTED`. The pinned `CORE-SEND-003` runner expects that send to succeed, so the TCK task deselects it.

This crate depends on `a2a-lf` 0.4.1, `a2a-server-lf` 0.5.1, `a2a-client-lf` 0.2.7, and `a2a-grpc` 0.3.

## MCP

`McpServer` targets MCP `2026-07-28` through `rmcp` 3.5. The tools are `list_log_sources`, `query_logs`, `list_metrics`, `query_metric`, `list_tasks`, `start_task`, and `get_task_status`. Input and output schemas come from the same request and result types used by A2A.

`serve_stdio` uses newline-delimited JSON-RPC on standard input and output. `serve_http` mounts Streamable HTTP at `/mcp`. With no listener, the MCP server binds `127.0.0.1:31001` and the A2A server binds `127.0.0.1:31000` after connecting to that MCP URL through `McpLab`. Starting a task waits until the run is completed, failed, or canceled. Set `wait` to false to return as soon as the run is accepted, then poll with `get_task_status`.

Provider failures become MCP invalid-params or internal errors. The same failures become A2A `google.rpc.Status` mappings (`invalid_params`, `task_not_found`, or `internal`).

The architecture is described in [Lab dev kit architecture](<../architecture/doc-1 - Lab-dev-kit-architecture.md>).
