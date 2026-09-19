# examples

Focused, runnable demos — one feature each. From this directory:

```sh
cargo run -p example-<name>
```

| Example | Feature flag | What it shows |
|---------|--------------|---------------|
| `example-sse` | — | Streaming responses / server-sent events via `Body::channel` |
| `example-sessions` | — | Cookie sessions: login → whoami → logout, auto `Set-Cookie` |
| `example-static-files` | — | `ServeDir` with cache headers, precompressed siblings, ranges |
| `example-middleware-tour` | `compression` | Every built-in middleware, one line each |
| `example-websocket` | `websocket` | WebSocket echo over `Router::websocket` |
| `example-tls` | `tls` | HTTPS with rustls; ALPN negotiates HTTP/2 |
| `example-hello` | — | A full application: routing, controllers, services, state |

The first four run with zero setup. `example-websocket` needs a WS client
(e.g. `npx wscat -c ws://127.0.0.1:3000/ws`). `example-tls` embeds the repo's
*test* certificates — swap in your own PEM files for anything real.

`example-hello` predates the others and doubles as an integration exercise:

```sh
ENV_NAME=dev
cargo run -p example-hello dev
```
