# Survey Tool Client<>Server Api

This folder is the **single source of truth** for the survey tool's client/server contract.
The same logical API is described for two transports:

- **gRPC** – [`grpc/survey_tool.proto`](grpc/survey_tool.proto) (proto3, package `survey.v1`)
- **OpenAPI** – [`openapi/survey-tool-api.yaml`](openapi/survey-tool-api.yaml) (OpenAPI 3)

Both describe the same operations and data model; the gRPC RPC comments even list the REST
route and the HTTP/gRPC status codes each operation can return, so the two files can be read
side by side.

## Domain model

| Type | Purpose |
| --- | --- |
| `SurveySummary` | Metadata + server-computed aggregates for a survey (counts, first/last submit time, min/max/avg score). Score fields are only meaningful for quizzes. |
| `SurveyResult` | One participant run: `origin`, `start_time`/`end_time`, optional `user`/`score` (quiz), answered page/question counts and the list of `QuestionAnswer`. |
| `QuestionAnswer` | A single answer, tagged with its `QuestionType`. The concrete value is a `oneof`/`oneOf` — exactly one of string / int / string-list / string-map / range is set, and which one is valid depends on the question type. |
| `HighscoreEntry` | `name`, `score`, `time` for a quiz's top results. |
| `SurveyType` | `survey` or `quiz`. |
| `QuestionType` | `data`, `choice`, `text`, `rating`, `likert`, `datetime`, `slider`. |

### Answer ↔ question-type pairing

The `answer` field is a union, but not every value is valid for every question type. The
server enforces this pairing and rejects mismatches:

| QuestionType | Expected answer variant |
| --- | --- |
| `data`, `text`, `datetime` | string |
| `rating` | int |
| `choice` | string list |
| `likert` | string map |
| `slider` | range (`first`, optional `second`) |

## Operations

The API is split into three services (gRPC) / route groups (REST):

**SurveyService** — survey lifecycle
- `ListSurveys` → `GET /v1/survey` — optional filter by type and/or active flag
- `CreateSurvey` → `POST /v1/survey` — upload a ZIP bundle, returns the new survey UUID
- `GetSurvey` → `GET /v1/survey/{id}` — download the original ZIP
- `DeleteSurvey` → `DELETE /v1/survey/{id}`
- `SetSurveyActive` → `PUT /v1/survey/{id}/active/{set}`

**SurveyResultsService** — participant results
- `GetResults` → `GET /v1/survey/{id}/results`
- `AddResult` → `POST /v1/survey/{id}/results`
- `DeleteResults` → `DELETE /v1/survey/{id}/results`

**SurveyDataService** — aggregates
- `GetSurveySummary` → `GET /v1/survey/{id}/summary`
- `GetHighscore` → `GET /v1/survey/{id}/highscore` (optional `limit`, default 10)

## Conventions & good to know

- **Survey bundle** – `CreateSurvey`/`GetSurvey` transport the raw ZIP as bytes. The server
  expects a `survey_config.yaml` in the root of the archive (best to create the zip via
  [survey-tool-cli](https://github.com/HuppiFluppi/survey-tool-cli)).
- **Timestamps** – all time fields are RFC 3339 strings
  (`{year}-{month}-{day}T{hour}:{min}:{sec}[.{frac_sec}]Z`). In gRPC they are carried as
  `google.protobuf.Timestamp`.
- **Auth** – the REST spec declares HTTP Basic auth (`securitySchemes.http`). The gRPC
  transport carries the same credentials as `user` / `pass` request metadata. Both are
  role-based (`ADMIN`, `USER`); see the server README for how roles map to operations.
  Credentials are sent in plaintext, so run behind TLS in any non-local setup.
- **Enum zero values** – the proto enums reserve `*_UNSPECIFIED = 0` per proto3 convention.
  `SURVEY_TYPE_UNSPECIFIED` / `QUESTION_TYPE_UNSPECIFIED` are not valid domain values and are
  rejected/treated as "no filter" by the server.

## Caveats

- **GET with a body** – `GET /v1/survey` currently models its filter as a JSON request body.
  Many HTTP clients and proxies do not support bodies on GET; this is flagged in the spec and
  is a candidate for moving to query parameters.
- **Spec drift** – the two files are maintained by hand and hence have a risk to diverge.

## Code generation

The recommended way to communicate with the server is to use the published client library (rust or kotlin).

Another option is to generate bindings from api files directly. For example, the Rust workspace compiles the proto at build time with
`tonic-prost-build` (see `rust/server/build.rs` and `rust/client/build.rs`), pointing at
`../../api/grpc/survey_tool.proto`. Check the respective client and server projects for how
each stack consumes the API.
