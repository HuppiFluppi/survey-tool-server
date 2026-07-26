# Survey Tool Server

A server (and matching client libraries) for distributing survey/quiz configurations and
collecting their results. It is the backend counterpart to [survey-tool gui](https://github.com/HuppiFluppi/survey-tool) and
[survey-tool-cli](https://github.com/HuppiFluppi/survey-tool-cli). Surveys are
authored as a configuration bundle, uploaded to the server, answered by participants, and
their results are stored and aggregated (summaries, highscores).

The project intentionally offers the **same API over two transports** (gRPC and REST) and
**multiple language implementations** (Rust today, Kotlin planned) so the different
stacks can be compared against each other. It is a learning and experimentation project —
see the disclaimer below.

## Concepts

- **Survey** – a configuration bundle (a ZIP containing a `survey_config.yaml`) that defines
  pages and questions. A survey is either of type `survey` or `quiz`.
- **Result** – one participant's run of a survey: timing, origin, optional user/score (for
  quizzes) and the individual answers per question.
- **Summary** – server-computed aggregates for a survey (submit count, first/last submit
  time, min/max/avg score).
- **Highscore** – the top scoring results of a quiz.
- **Active flag** – inactive surveys reject new results and are hidden from non-admin
  callers.

## Repository contents

| Path | Description |
| --- | --- |
| [`api/`](api) | Transport-agnostic API definitions: gRPC (`.proto`) and OpenAPI (`.yaml`). The single source of truth for all client/server implementations. See [api/README.md](api/README.md). |
| [`rust/`](rust) | Rust workspace with the server binary and a reusable client library. Currently the reference implementation. See [rust/README.md](rust/README.md). |
| `kotlin/` | Planned Ktor-based server and client (scaffolding only for now). |
| `infrastructure/` | Infrastructure-as-code for AWS deployments (`aws-infra-only` and `container` variants). Work in progress. |
| `.github/workflows/` | CI pipelines (`ci-rust`, `ci-kotlin`) and the release workflow. |

## Getting started

### From source

The Rust server is the ready-to-run component. To build and start it with local (SQLite +
filesystem) persistence:

```bash
cd rust
cargo run -p survey-tool-server
```

This starts a gRPC server on `127.0.0.1:1504` with authentication disabled. For all
configuration options (auth, TLS, persistence backends) see [rust/README.md](rust/README.md).

### Via Container/Docker

Container for all server variants are published to the Github container registry.
Configuration can be done via environment parameters.

## Roadmap

- [x] (API) Publish REST & gRPC api
- [x] (Rust) Create grpc server
- [x] (Rust) Create grpc client
- [] (Rust) Provide Docker build
- [] (Rust) Add Rest server
- [] (Rust) Add Rest client
- [] (Rust) Add AWS persistence option
- [] (Rust) Improve logging
- [] (Kotlin) Create Ktor server
- [] (Kotlin) Create Ktor client
- [] (Kotlin) Provide Docker build
- [] Add IaC files for AWS (CDK + CF)
- [] Create benchmark
- [] Try AWS services only infrastructure
- [] Add streaming endpoints

## Contributing
Contributions are welcome! Please:
- Open issues with clear steps to reproduce and expected behavior
- Submit Pull Requests with concise descriptions, tests (where applicable), and clean commit history
- Follow the project’s and respective language coding style and patterns

## Disclaimer
This software is provided "as is", without warranty of any kind. The author is certain, parts of this software could be done better.
The todos are plenty and bugs are likely hiding. Use at your own risk and have fun. This is a learning and experiment project.

## License
This project is provided under MIT license. See [license file](LICENSE)
