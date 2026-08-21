# R0 External Contract Record

Status: `VERIFIED_FOR_V1_0_IMPLEMENTATION`

Verified on 2026-08-18 for CAO task `grok-codex-bridge-v1-0`, epoch `e1`. This record fixes the external facts consumed by Phase A through Phase E. It does not authorize V1.1, a live quota-bearing request, Codex configuration mutation, or credential mutation.

## Source snapshots

- Local Codex runtime: `codex-cli 0.148.0-alpha.15` from the current ChatGPT application bundle.
- OpenAI Codex source: [`openai/codex@3d47dc40be53a2b7729443cf89cfa8d058830aa1`](https://github.com/openai/codex/tree/3d47dc40be53a2b7729443cf89cfa8d058830aa1).
- xAI Grok Build source: [`xai-org/grok-build@d71f6e0c1f5acc5469e503e192fe14824e6f8c90`](https://github.com/xai-org/grok-build/tree/d71f6e0c1f5acc5469e503e192fe14824e6f8c90).
- Current public contracts: [Codex configuration reference](https://developers.openai.com/codex/config-reference/), [Codex advanced configuration](https://developers.openai.com/codex/config-advanced/), [Grok Build overview](https://docs.x.ai/build/overview), [Grok Build enterprise deployment](https://docs.x.ai/build/enterprise), [xAI text generation](https://docs.x.ai/developers/model-capabilities/text/generate-text), [xAI streaming](https://docs.x.ai/developers/model-capabilities/text/streaming), and [xAI function calling](https://docs.x.ai/developers/tools/function-calling).

## Codex provider boundary

Current Codex custom providers use these user-level fields:

- `model_provider = "grok_bridge"` selects the provider.
- `[model_providers.grok_bridge].base_url` is the API base; Codex appends `responses` for the HTTP stream request.
- `wire_api = "responses"` is the only supported wire API and is also the default.
- `requires_openai_auth = false` prevents Codex from applying OpenAI login credentials to this provider.
- `supports_websockets = false` keeps V1.0 on HTTP SSE.
- Provider/auth redirection fields are ignored in project-local config. V1.0 must use user config or a user profile.
- Current profiles are top-level overlays at `$CODEX_HOME/<name>.config.toml`, selected with `--profile <name>`. Legacy `[profiles.<name>]` tables are not current syntax.

The V1.0 capability token therefore belongs in the loopback `base_url` path. It is local caller authentication, not an OpenAI or xAI credential. Native GPT configuration remains outside this provider and profile.

## Codex Responses request

The current Codex HTTP request type contains `model`, `instructions`, `input`, optional `tools`, `tool_choice`, `parallel_tool_calls`, optional `reasoning`, `store`, `stream`, optional `stream_options`, `include`, optional `service_tier`, optional `prompt_cache_key`, optional `text`, and optional `client_metadata`. The source contract is [`ResponsesApiRequest`](https://github.com/openai/codex/blob/3d47dc40be53a2b7729443cf89cfa8d058830aa1/codex-rs/codex-api/src/common.rs).

The V1.0 items that must survive the bridge are:

- messages with ordered `input_text` and `input_image` content;
- `input_image.image_url`, including data URLs, plus optional `detail`;
- function definitions without name, description, or JSON Schema rewriting;
- function calls with stable `call_id`, `name`, and JSON arguments;
- `function_call_output` with the same `call_id` and either a string or ordered content items;
- tool-result content items `input_text`, `input_image`, and any explicitly supported current Codex item; unsupported items fail explicitly instead of being removed;
- multiple calls and outputs in their original order when `parallel_tool_calls` is enabled.

Codex's current protocol types are authoritative in [`codex-rs/protocol/src/models.rs`](https://github.com/openai/codex/blob/3d47dc40be53a2b7729443cf89cfa8d058830aa1/codex-rs/protocol/src/models.rs).

## Codex SSE consumption

Codex accepts standard Responses SSE. The events needed by its current consumer include:

- `response.created`;
- `response.output_item.added` and `response.output_item.done`;
- `response.output_text.delta`;
- `response.completed`;
- `response.failed` and `response.incomplete` as terminal failures.

It tolerates standard lifecycle events such as content-part, output-text-done, and function-argument delta/done events. Function calls are completed from the output item, so the final `response.output_item.done.item` must be preserved exactly. The current parser is [`codex-api/src/sse/responses.rs`](https://github.com/openai/codex/blob/3d47dc40be53a2b7729443cf89cfa8d058830aa1/codex-rs/codex-api/src/sse/responses.rs).

## Grok OAuth and upstream boundary

For an official Grok Build login, the session inference host is `cli-chat-proxy.grok.com`; `auth.x.ai` owns login/OIDC; `api.x.ai` is the separately billed API-key route. V1.0 uses only the session inference route.

- Base URL: `https://cli-chat-proxy.grok.com/v1`.
- Inference endpoint: `POST /responses`.
- Model catalog endpoint: `GET /models`.
- Stream transport: SSE with `stream: true` and `Accept: text/event-stream`.
- Credential source: `$GROK_AUTH_PATH` when set, otherwise `$GROK_HOME/auth.json`, with `$GROK_HOME` falling back to `~/.grok`. If `$GROK_AUTH_PATH` points outside the official home, `$GROK_HOME` must also identify that home so the bridge can resolve its official CLI helper.
- `auth.json` is a scope-to-record JSON map. The selected current session record contains at least `key`, `auth_mode`, `create_time`, and `user_id`, with optional expiry and profile fields. V1.0 inspects it in place and never handles a refresh token or rewrites it. If `expires_at` is absent, the bridge uses `create_time + 30 days` as a parser fallback. When the selected record is hard-expired during a provider request, the bridge may invoke the official `bin/grok models` command non-interactively once, with a 7-second timeout and disconnected standard streams; the official process owns any silent OIDC refresh, and the bridge only rereads the file afterward for up to the 60-second request grace.
- A hard-expired access token is not sent. Missing, expired, 401, or 403 state stops with guidance to use the official `grok login` path.

The official source attaches `Authorization: Bearer <session token>`, `X-XAI-Token-Auth: xai-grok-cli`, `x-authenticateresponse: authenticate-response`, `x-grok-client-mode`, truthful request/conversation identifiers, and `x-grok-model-override`. The bridge will identify itself truthfully as `grok-codex-bridge`; it will not impersonate the Grok Build User-Agent or client identifier. Redirects may not carry the bearer to another origin.

## Responses-to-Responses decision

The current Grok Build catalog selects `api_backend: "responses"` for `grok-4.6` and `grok-4.5`, and its official sampler posts to `/responses`. xAI also documents Responses as its preferred REST API. Therefore V1.0 is a Responses-to-Responses bridge.

The superseded candidate `/v1/chat/completions`, model slug `grok-build`, and Responses-to-Chat-Completions conversion are not active implementation contracts. The bridge may validate, constrain, and normalize transport metadata, but it must not translate a current Responses request into the legacy chat protocol.

`store` is forced to `false` unless a higher current requirement explicitly changes the privacy contract. Hosted `web_search` and `x_search` are not injected by the bridge.

## Grok model evolution contract

V1.0 must not compile one permanent Grok model slug into its routing logic.

- The source snapshot supplies a bootstrap last-known-good catalog containing current `grok-4.6` and `grok-4.5` Responses models.
- Once Phase B owns credentialed transport, a bounded refresh reads the official session endpoint `GET https://cli-chat-proxy.grok.com/v1/models` using the same origin-scoped session authentication.
- Only entries with a non-empty stable identifier/model, a Responses backend, and an allowed xAI inference base are admitted. Hidden, unsupported, malformed, legacy-protocol, or alternate-origin entries are rejected.
- Refresh replacement is atomic and origin-scoped. An empty, malformed, unauthorized, or failed response leaves the last-known-good catalog unchanged.
- The bridge exposes the current admitted catalog through its own capability-scoped `/v1/models`; a request naming a model outside that catalog fails explicitly.
- Startup may perform one bounded catalog refresh, and `catalog refresh` provides an explicit refresh. Both catalog paths require a currently usable credential and use direct credential loading; neither invokes the hard-expiry renewal helper. That helper is limited to the Responses provider request path. V1.0 does not add an unattended polling loop.

This lets a future officially published model such as `grok-4.7` become available after a successful official-catalog refresh without a bridge code release. It does not add the model to the native Codex picker; that remains V1.1 and is excluded.

## Tool, image, and stream compatibility

xAI's current Responses path uses the same function tool shape (`type`, `name`, `description`, `parameters`), `function_call`/`function_call_output` pairing by `call_id`, parallel calls, `input_image` URL/data-URL content, and structured tool-result content. Current Grok Build source exercises image-bearing `function_call_output` as ordered `input_text` plus `input_image` parts.

Phase C and Phase D should therefore implement strict typed validation and lossless pass-through/canonicalization rather than cross-protocol synthesis. Phase E proves the image path. A schema mismatch observed against an admitted fixture is cause-bound evidence for a narrow adapter; no speculative fallback is authorized.

## Reference licenses

- OpenAI Codex snapshot: Apache-2.0.
- xAI Grok Build snapshot: Apache-2.0 for first-party code; its third-party notices remain separate.
- [`duolahypercho/codex-router`](https://github.com/duolahypercho/codex-router): MIT. It is a reference for isolation, rollback, and capability routing only.

No reference implementation code was copied into this repository during R0. The repository's publication license remains a separate user decision.

## Remaining live boundaries

- R0 did not read the user's real Grok credential, call the live model catalog, consume inference quota, mutate Codex configuration, or activate a service.
- Phase B must prove strict catalog/auth parsing with fixtures before any explicitly authorized live check.
- Phase G owns the first quota-bearing end-to-end proof and the exact live entitlement result.
