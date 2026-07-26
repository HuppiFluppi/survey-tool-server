# Survey Tool Server — Rust

The Rust reference implementation of the [survey tool API](../api). It is a Cargo workspace
containing the server binary and a reusable client library. Today it speaks **gRPC** and
persists to the **local filesystem + SQLite**; REST and AWS backends are scaffolded but not
yet implemented (see [Caveats](#caveats)).

## Workspace layout

| Crate / path        | Description                                                                                     |
|---------------------|-------------------------------------------------------------------------------------------------|
| [`server/`](server) | `survey-tool-server` binary. Serves the gRPC API and owns the persistence layer.                |
| [`client/`](client) | `survey-tool-api-client` library. A thin, typed async wrapper around the generated gRPC client. |
| [`test/`](test)     | Cross-crate / integration tests.                                                                |

Both crates generate their gRPC bindings at build time from `../../api/grpc/survey_tool.proto`
via `tonic-prost-build` (see the respective `build.rs`).

## Building & running (server)

```bash
# from the rust/ directory
cargo build
cargo run -p survey-tool-server
```

The default build enables the `grpc` and `local` features and starts a gRPC server on
`127.0.0.1:1504` with **authentication disabled**.

> **Note:** the server depends on `survey-tool-cli` via a git dependency, so the first build
> needs network access.

### Cargo features (server)

| Feature | Default | Status | Effect |
| --- | --- | --- | --- |
| `grpc` | ✅ | working | gRPC transport. |
| `local` | ✅ | working | SQLite + filesystem persistence (pulls in `rusqlite`, bundled SQLite). |
| `rest` | – | **stub** | Placeholder module only; does not compile as a server yet. |
| `aws` | – | **stub** | Placeholder for S3 + DynamoDB persistence; not implemented. |

The client crate exposes `grpc` (default) and a placeholder `rest` feature.

## Configuration (server)

All options are available as CLI flags **and** environment variables (uppercased field name).
Run `cargo run -p survey-tool-server -- --help` for the authoritative list. The most relevant:

| Flag | Env | Default | Description |
| --- | --- | --- | --- |
| `--grpc-address` | `GRPC_ADDRESS` | `127.0.0.1:1504` | gRPC listen address. |
| `--auth-setting` | `AUTH_SETTING` | `none` | `none` or `simple`. |
| `--auth-config` | `AUTH_CONFIG` | – | Required when auth is `simple` (see below). |
| `--tls-setting` | `TLS_SETTING` | `off` | `off` or `pem`. |
| `--tls-cert-pem-file` / `--tls-key-pem-file` | `TLS_CERT_PEM_FILE` / `TLS_KEY_PEM_FILE` | – | Required when TLS is `pem`. |
| `--persistence-type` | `PERSISTENCE_TYPE` | `local` | `local` (or `aws` when built with that feature). |
| `--persistence-local-storage-folder` | `PERSISTENCE_LOCAL_STORAGE_FOLDER` | `./sts/files/` | Where survey ZIPs are stored. |
| `--persistence-local-db-folder` | `PERSISTENCE_LOCAL_DB_FOLDER` | `./sts/db/` | Where the SQLite database lives. |
| `--persistence-local-no-create` | `PERSISTENCE_LOCAL_NO_CREATE` | off | If set, the folders must already exist (they are not auto-created). |

### Authentication & authorization

`--auth-setting simple` enables role-based auth. Credentials are supplied with:

> **Note:** the simple authentication implemented is NOT suitable for production use. 
> Without TLS, it transmits user and password in plaintext!

```
--auth-config 'user1:pass1:role1,role2;user2:pass2:role1,role2'
```

- Entries are separated by `;`, fields by `:`, and roles by `,`.
- Valid roles are `Admin` and `User` (case-sensitive).
- Over gRPC the client sends the chosen credentials as `user` / `pass` request metadata.

Role requirements per operation:

| Role    | Operations                                                                                  |
|---------|---------------------------------------------------------------------------------------------|
| `User`  | list surveys, get an **active** survey, get summary, get highscore, add a result            |
| `Admin` | create / delete a survey, set active flag, get an **inactive** survey, get / delete results |

> **Good to know**
> - Credentials travel in plaintext metadata, so combine `simple` auth with `pem` TLS for any
>   non-local deployment.
> - A password containing `:` cannot be expressed in `--auth-config` (the `:` is the field
>   delimiter).
> - Roles are not including *lower* roles. That means Admin role is not including User and reading a survey requires the `User` role even for admins. 
> - Grant `Admin,User` to accounts that need both.

### Local persistence details

- Each uploaded survey ZIP is stored as a file named by its UUID under the storage folder.
- Survey metadata, results and answers live in `survey-tool-server.sqlite` under the db folder.
- On upload the server opens the ZIP, reads `survey_config.yaml` from its root and indexes the
  title, description, type, page/question counts and whether it contains conditional elements.
- The whole ZIP is held in memory during upload/download and there is currently **no upload
  size limit** — do not expose the server to untrusted uploads without a proxy that caps
  request size.

## Using the client library

```rust
use survey_tool_api_client::grpc::{SurveyApiClient, GrpcAuthSetting};

// No auth, no TLS
let mut client = SurveyApiClient::new("http://127.0.0.1:1504").await.unwrap();

// With auth over TLS (native root certificates)
let mut client = SurveyApiClient::with_options(
    "https://my-server:1504",
    GrpcAuthSetting::Simple { user: "alice".into(), pass: "secret".into() },
).await.unwrap();

let surveys = client.list_surveys(None, None).await.unwrap();
```

- `new` connects in plaintext; `with_options` enables TLS with the system's native roots and
  attaches the configured credentials to every request.
- Errors are returned as `SurveyApiClientError`, which maps gRPC status codes to typed
  variants (`GrpcNotFound`, `GrpcUnauthenticated`, `GrpcPermissionDenied`, …).

## Testing

```bash
cargo test
```

The persistence tests run against an in-memory SQLite database and temporary directories, so
they need no external services.

## Caveats

- **`rest` and `aws` are not implemented.** Building with `--features rest` or `--features
  aws` will fail: the binary references server/persistence entry points that these modules
  don't provide yet.
- Highscore queries assume quiz results (non-null `user`/`score`); running them against a
  plain survey won't provide meaningful values.
- This is a learning project — see the disclaimer in the [root README](../README.md).
