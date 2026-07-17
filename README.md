# clacks

A Tauri desktop app that bridges Telegram to two subscription-based interactive Claude CLI sessions, arranged as a dual-LLM security pipeline:

- **taster** (the sanitizer): zero tools, no memory, no network. It inspects untrusted incoming Telegram messages and emits a strict JSON verdict — nothing else.
- **cyrano** (the responder): only ever sees sanitized text, with read-only access to a whitelisted project directory, and drafts the reply.
- The **Rust backend** is the sole mediator: it polls Telegram, feeds the PTYs, watches Stop-hook outboxes, and sends replies. Neither CLI can touch the bot token.

The name comes from Terry Pratchett's *Going Postal*: the clacks are a network of message relay towers, and the Woodpecker is a message that destroys the tower processing it — the literary prototype of prompt injection. The taster exists so the Woodpecker never gets past the first tower.

## Status

Design phase. See the design document (zh-TW): [docs/superpowers/specs/2026-07-17-clacks-design.md](docs/superpowers/specs/2026-07-17-clacks-design.md)

## Planned stack

Tauri 2 + Rust (portable-pty, notify, rusqlite) · React 18 + TypeScript + Vite + xterm.js

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
