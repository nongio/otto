# Clients

This repository ships official client libraries for five languages.
Every client's wire types are generated from the canonical TypeScript
sources in [`types/`](https://github.com/microsoft/agent-host-protocol/tree/main/types),
so the same protocol shape is presented idiomatically in each
ecosystem. Each library is versioned and released independently — see
[`RELEASING.md`](https://github.com/microsoft/agent-host-protocol/blob/main/RELEASING.md)
for the per-language tag scheme.

Pick the language you want to integrate against and install from the
package registry it normally ships through.

| Language       | Package                                                  | Registry        |
| -------------- | -------------------------------------------------------- | --------------- |
| [Rust](#rust)             | `ahp-types`, `ahp`, `ahp-ws`                  | crates.io       |
| [TypeScript](#typescript) | `@microsoft/agent-host-protocol`              | npm             |
| [Kotlin / JVM](#kotlin)   | `com.microsoft.agenthostprotocol:agent-host-protocol` | Maven Central |
| [Swift](#swift)           | `AgentHostProtocol`, `AgentHostProtocolClient` | Swift Package Manager |
| [Go](#go)                 | `github.com/microsoft/agent-host-protocol/clients/go` | Go module proxy |

## Rust

Three crates on [crates.io](https://crates.io), mirroring the split
between wire types, transport-agnostic client, and a WebSocket
adapter:

- [`ahp-types`](https://crates.io/crates/ahp-types) — generated wire
  types, no I/O.
- [`ahp`](https://crates.io/crates/ahp) — async `Client` with pure
  reducers, a pluggable `Transport` trait, and the multi-host
  registry under `ahp::hosts`.
- [`ahp-ws`](https://crates.io/crates/ahp-ws) — WebSocket transport
  built on `tokio-tungstenite`.

```bash
cargo add ahp ahp-ws
# add `ahp-types` directly only if you need the wire types without
# the client runtime.
```

See the [Rust client README](https://github.com/microsoft/agent-host-protocol/tree/main/clients/rust)
for the quick start, custom transports, and multi-host details.

## TypeScript

A single browser- and Node-friendly package on
[npm](https://www.npmjs.com/package/@microsoft/agent-host-protocol)
with four subpath entry points (wire types, client, multi-host
orchestration, WebSocket transport):

[![npm](https://img.shields.io/npm/v/@microsoft/agent-host-protocol.svg)](https://www.npmjs.com/package/@microsoft/agent-host-protocol)

```bash
npm install @microsoft/agent-host-protocol
```

```ts
import { AhpClient } from '@microsoft/agent-host-protocol/client';
import { WebSocketTransport } from '@microsoft/agent-host-protocol/ws';
```

See the [TypeScript client README](https://github.com/microsoft/agent-host-protocol/tree/main/clients/typescript)
for the full subpath table and a complete quick start.

## Kotlin

Pure Kotlin/JVM artifact on
[Maven Central](https://central.sonatype.com/artifact/com.microsoft.agenthostprotocol/agent-host-protocol).
Targets Java 8 bytecode, so it can be consumed unchanged from
Android, server-side JVM services, and KMP/JVM targets.

[![Maven Central](https://img.shields.io/maven-central/v/com.microsoft.agenthostprotocol/agent-host-protocol)](https://central.sonatype.com/artifact/com.microsoft.agenthostprotocol/agent-host-protocol)

### Gradle (Kotlin DSL)

```kotlin
dependencies {
    implementation("com.microsoft.agenthostprotocol:agent-host-protocol:0.2.0")
}
```

### Gradle (Groovy DSL)

```groovy
dependencies {
    implementation 'com.microsoft.agenthostprotocol:agent-host-protocol:0.2.0'
}
```

### Maven

```xml
<dependency>
    <groupId>com.microsoft.agenthostprotocol</groupId>
    <artifactId>agent-host-protocol</artifactId>
    <version>0.2.0</version>
</dependency>
```

The library transitively depends on `org.jetbrains.kotlinx:kotlinx-serialization-json`.
See the [Kotlin client README](https://github.com/microsoft/agent-host-protocol/tree/main/clients/kotlin)
for usage, the `Ahp.json` serializer instance, and what is/isn't
included in the box (no transport yet — bring your own OkHttp/Ktor).

## Swift

Distributed via Swift Package Manager. The `Package.swift` manifest
lives at the repository root because SwiftPM only resolves manifests
at the root of a remote git repo; the Swift sources themselves live
under `clients/swift/AgentHostProtocol/`.

### Package.swift dependency

```swift
.package(url: "https://github.com/microsoft/agent-host-protocol.git", from: "0.1.0")
```

### Target dependencies

```swift
.target(
    name: "MyApp",
    dependencies: [
        .product(name: "AgentHostProtocol", package: "agent-host-protocol"),
        .product(name: "AgentHostProtocolClient", package: "agent-host-protocol"),
    ]
)
```

- `AgentHostProtocol` — generated wire types, commands, notifications,
  actions, and pure reducers.
- `AgentHostProtocolClient` — single-host `AHPClient`, `MultiHostClient`,
  state mirrors, and the `URLSessionWebSocketTransport` /
  `NWConnectionWebSocketTransport` transports.

See the [Swift client README](https://github.com/microsoft/agent-host-protocol/tree/main/clients/swift/AgentHostProtocol)
for the minimal single-host and multi-host examples, transport
choices, and reconnect layering guidance.

## Go

Single Go module resolved through the public Go module proxy. Three
packages mirror the Rust three-crate split:

- `ahptypes` — wire protocol types only.
- `ahp` — async `Client` over a pluggable `Transport`, pure reducers,
  and the multi-host runtime under `ahp/hosts`.
- `ahpws` — WebSocket transport built on
  [`github.com/coder/websocket`](https://github.com/coder/websocket).

```bash
go get github.com/microsoft/agent-host-protocol/clients/go@latest
```

```go
import (
    "github.com/microsoft/agent-host-protocol/clients/go/ahp"
    "github.com/microsoft/agent-host-protocol/clients/go/ahptypes"
    "github.com/microsoft/agent-host-protocol/clients/go/ahpws"
)
```

See the [Go client README](https://github.com/microsoft/agent-host-protocol/tree/main/clients/go)
for the WebSocket quick start.

## Picking a protocol version

Every client exposes two protocol-version constants:

- `PROTOCOL_VERSION` — the SemVer string for the version that
  client's current release implements.
- `SUPPORTED_PROTOCOL_VERSIONS` — every version the client is
  willing to negotiate, most-preferred-first. Pass it (or a
  derived list/array) as `protocolVersions` on `InitializeParams`
  during the handshake.

Client package versions track the spec version, but they are not
required to match exactly — a client released after a patch-only
client fix may sit at a higher patch than the matching spec
release. The two version constants above always reflect the
protocol versions that build of the client can speak. See
[Versioning](/specification/versioning) for the full negotiation
model.
