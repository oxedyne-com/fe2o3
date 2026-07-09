# Red — TODO

> Last updated: 2026-07-09
> Priority: P0 = MVP blocker, P1 = soon after MVP, P2 = future

## 1. MVP Polish

Things that are broken or missing from the current chat experience.

- [ ] **P0 — True incremental SSE streaming.** Currently `do_request` reads the entire LLM response into a `Vec<u8>` buffer, then `parse_sse_stream` processes it. The user sees nothing until the full response arrives. Fix: read the TLS stream line-by-line, parse each `data:` line as it arrives, call `on_token` immediately. This is the single biggest UX gap.
- [ ] **P0 — Add `max_tokens` to LLM requests.** GLM-5.2 sometimes enters long reasoning loops (repeated text, thousands of tokens). Set a sensible default (e.g. 4096) and make it configurable.
- [ ] **P0 — Stop/abort button.** When the model is generating, show a stop button that closes the LLM connection and sends `done` to the client. The server needs to be able to cancel the in-flight `chat_stream` future.
- [ ] **P0 — WebSocket reconnection.** If the chat WS drops mid-conversation (network blip, server restart), the client should auto-reconnect and re-authenticate, preserving the current session and chat history. Currently a dropped WS means a blank page until manual refresh.
- [ ] **P1 — Remove verbose `info!` logging from `llm.rs`.** Every `stream.read()` chunk logs at `info!` level. Move to `debug!` or remove. Same for `text_msg` debug logging added during development.
- [ ] **P1 — Favicon.** Currently 404. Add a small Red/Oxedyne favicon.
- [ ] **P1 — Session timestamps.** Session boxes should show relative timestamps (e.g. "2h ago") so users can identify sessions by when they were created.
- [ ] **P1 — Empty state.** When no session is active, the chat area shows nothing. Show a welcome message with model info and a "Start a new session" prompt.
- [ ] **P2 — Keyboard shortcuts.** Ctrl+N for new session, Ctrl+K for session search/switch, Esc to cancel.
- [ ] **P2 — Copy button on code blocks.** Markdown code blocks should have a copy-to-clipboard button.

## 2. Multi-Provider Support

The settings modal has a provider dropdown but only Fireworks.ai is wired. To be a real product, users need to use their own keys with multiple providers.

- [ ] **P1 — Provider abstraction in `llm.rs`.** `LlmClient` is hardcoded to Fireworks.ai's endpoint format. Abstract the provider: different base URLs, auth header formats, model naming conventions. A `Provider` enum or trait with `fireworks`, `openai`, `anthropic`, `google`, `groq` variants.
- [ ] **P1 — Per-user API key storage.** Users enter their own API key in settings, stored encrypted in O3db. The server uses the user's key instead of the global `red_config.llm_key`. This is the BYOK (bring your own key) model.
- [ ] **P1 — Model list per provider.** Each provider has different available models. Fetch the model list dynamically from the provider's `/v1/models` endpoint, or maintain a curated list. The model picker in the new-session panel should show only models for the selected provider.
- [ ] **P2 — Provider-specific pricing.** The pricing table is hardcoded for Fireworks. Each provider has different pricing. Fetch from provider API or maintain a pricing config.
- [ ] **P2 — Fallback/retry.** If one provider is down, optionally retry with a fallback provider.

## 3. Billing & Usage

For a commercial offering with per-user billing.

- [ ] **P1 — Per-user token usage aggregation.** Currently tokens are tracked per-session. Add a per-user aggregate: total tokens, total cost, broken down by model and day. Store in O3db under `user:<username>:usage`.
- [ ] **P1 — Usage display in settings.** Show a summary of the user's total usage (tokens, cost, sessions) in the settings modal. A simple table, not a full dashboard.
- [ ] **P2 — Spending limits.** Configurable per-user spending limit. When exceeded, block new chat messages with an informative error. Requires server-side enforcement.
- [ ] **P2 — Billing dashboard.** Daily/weekly/monthly usage charts. Session-level breakdown. Export as CSV.
- [ ] **P2 — Admin panel.** View all users, their usage, suspend/activate accounts, set spending limits. Steel admin command or separate admin UI.

## 4. Tools & Sandbox (Phase 2 from plan)

Give the agent the ability to execute code and manipulate files.

- [ ] **P1 — `RedTool` trait + `ToolRegistry`.** Define the trait with `name()`, `description()`, `parameters_schema()`, `execute()`. Registry holds registered tools and provides definitions for the LLM API request.
- [ ] **P1 — Sandbox directory management.** Per-session working directory under `sessions/<user>/<session_id>/`. Path traversal protection in `resolve()`. Auto-cleanup on session close.
- [ ] **P1 — Shell tool.** Execute shell commands in the sandbox directory via `std::process::Command`. Capture stdout+stderr. Timeout after configurable period.
- [ ] **P1 — File tools.** `file_read`, `file_write`, `file_list`, `file_delete` — all confined to the sandbox directory.
- [ ] **P1 — Agent loop update.** Handle `tool_calls` in the LLM response. Execute tools, append `ChatMessage::Tool` results, re-send to LLM, loop until no more tool calls.
- [ ] **P1 — UI: tool call/result display.** Show tool calls (name, args) and results in the chat output, visually distinct from regular text. Collapsible.
- [ ] **P2 — Sandbox hardening.** Restricted PATH, unset dangerous env vars, resource limits (CPU, memory, disk). Optional `bubblewrap`/`firejail` integration.
- [ ] **P2 — Custom tools via config.** Enable/disable tools per vhost or per user. Custom tool parameters via `RedConfig`.

## 5. File Upload/Download (Phase 3 from plan)

- [ ] **P2 — Upload API handler.** `POST /api/upload` with multipart form data. Store in current session's sandbox. Requires auth.
- [ ] **P2 — Download API handler.** `GET /api/download/<filename>`. Serve from sandbox. Requires auth.
- [ ] **P2 — File panel in UI.** Sidebar or tab showing files in the current session's sandbox. Upload button, download links, file preview, delete.

## 6. Context & Conversation Management

- [ ] **P1 — Context window awareness.** Each model has a context window limit (e.g. 128k tokens). Track the total tokens in the conversation. When approaching the limit, warn the user or auto-summarise old messages.
- [ ] **P1 — Conversation export.** Download the full conversation as markdown or JSON. Useful for sharing or archiving.
- [ ] **P2 — Conversation search.** Search across all sessions for keywords.
- [ ] **P2 — Message editing.** Allow editing a previous user message and re-running from that point. Requires truncating conversation history and re-sending.
- [ ] **P2 — Branch/fork.** Create a new session from a point in an existing conversation, preserving history up to that point.

## 7. UI/UX

- [ ] **P1 — Markdown rendering improvements.** Tables, syntax highlighting in code blocks, inline LaTeX/math. The current `marked.min.js` handles basic markdown but not syntax highlighting.
- [ ] **P1 — Mobile responsiveness.** The sidebar should auto-collapse on mobile with a hamburger toggle. Chat input should adapt to viewport. Test on 375px width.
- [ ] **P2 — Session search/filter.** Filter sessions by name or model in the sidebar. Especially important when users have many sessions.
- [ ] **P2 — Drag-to-resize chat input.** The textarea auto-grows but should also be manually resizable.
- [ ] **P2 — Message timestamps.** Show timestamps on hover for each message.
- [ ] **P2 — User avatar/initials.** Show user initials in the sidebar footer instead of just the username text.
- [ ] **P2 — Branding.** Custom loading screen, better empty states, smooth transitions.

## 8. Security & Reliability

- [ ] **P1 — Rate limiting.** Per-user request rate limit on the chat WS endpoint. Prevent abuse. Steel already has `auth_guard` for HTTP rate limiting — extend to WS.
- [ ] **P1 — API key encryption at rest.** If storing user API keys in O3db, ensure they're encrypted with the user's passphrase (not just the database master key).
- [ ] **P1 — Session cookie security.** Verify HttpOnly, Secure, SameSite attributes on the session cookie. Check cookie expiry handling.
- [ ] **P2 — Audit log.** Log all chat requests, tool executions, and file operations for security review. Stored per-user in O3db.
- [ ] **P2 — Steel as systemd service.** Currently started via fragile tmux wrapper. A proper systemd service with the wallet passphrase handled by an orchestration layer (e.g. systemd-tty or a small unlock helper).

## 9. Extensibility (Phase 5 from plan)

- [ ] **P2 — MCP bridge.** Spawn MCP servers as subprocesses, bridge stdio to tool calls. Allows using any MCP-compatible tool (filesystem, browser, database, etc.) with Red.
- [ ] **P2 — Custom system prompts.** Per-session or per-user system prompts. Users can define their agent's persona.
- [ ] **P2 — Plugin system.** Allow third-party tools to be registered via config. Dynamic loading or compiled-in.
- [ ] **P2 — Webhook/API access.** Expose Red's agent via HTTP API for programmatic access (not just the web UI). Useful for integration with other systems.

## 10. Testing & CI

- [ ] **P1 — Unit tests for `llm.rs`.** Test SSE parsing with sample data (including `reasoning_content`, chunked encoding, `[DONE]`). Test `extract_json_string` with edge cases. Test `find_json_object` and `extract_json_number`.
- [ ] **P1 — Unit tests for `protocol.rs`.** Test `Session::to_datmap`/`from_datmap` round-trip with token counts. Test `ChatMessage` serialisation.
- [ ] **P2 — Integration test for agent loop.** Mock LLM client that returns canned responses. Test full agent turn including tool calls.
- [ ] **P2 — E2E browser test.** Automated Playwright test: login, create session, send message, verify response. Run against a local dev instance.
