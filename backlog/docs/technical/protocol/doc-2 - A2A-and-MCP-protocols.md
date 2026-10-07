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

The seven skills are:

- `list-log-sources`
- `query-logs`
- `list-metrics`
- `query-metric`
- `list-tasks`
- `start-task`
- `get-task-status`

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

`A2aServer::with_message_handler` adds an eighth skill, `agent-message`.

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

The card then keeps the seven lab skills.

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

Query-log and query-metric pages become one artifact chunk per item.

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

The tools are `list_log_sources`, `query_logs`, `list_metrics`, `query_metric`, `list_tasks`, `start_task`, and `get_task_status`.

Input and output schemas come from the same request and result types used by A2A.

`serve_stdio` uses newline-delimited JSON-RPC on standard input and output.

`serve_http` mounts Streamable HTTP at `/mcp`.

Accepted `Host` values are `localhost`, `127.0.0.1`, `::1`, and `host.docker.internal`.

With no listener, the MCP server binds `127.0.0.1:31001`.

With no listener, `A2aServer::listen` binds `127.0.0.1:31000`.

`McpLab::connect_default` calls `http://127.0.0.1:31001/mcp`.

`start_task` waits until the run is terminal unless `wait` is false.

`timeout_seconds` defaults to 60.

Set `wait` to false to return when the run is accepted.

Poll that run with `get_task_status`.

`invalid`, `protocol`, and `not_found` become MCP invalid-params errors.

`unavailable` and `transport` become internal errors.

Error data includes the `A2aLabError` code.

The architecture is described in [A2A-LAB devkit architecture](<../architecture/doc-1 - Lab-dev-kit-architecture.md>).
