# api

To install dependencies:

```bash
bun install
```

To run:

```bash
bun run start
```

The Effect HTTP API listens at `http://127.0.0.1:3000`.

- `GET /health` returns `{ "status": "ok" }`.
- `GET /hello/Arya` returns `{ "message": "Hello, Arya!" }`.
