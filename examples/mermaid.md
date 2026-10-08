# Mermaid previews

Open this file in Vaayu and press `,mp` for full-screen preview or `,ms` for a
split. Use `j/k` to scroll, `h/l` to pan, `+/-` to zoom graphical diagrams, and
`0` to reset the zoom and pan. Auto mode uses graphical diagrams when the
terminal answers the graphics probe; other terminals show Unicode diagrams.

## Request flow

```mermaid
flowchart LR
    Client([Client]) --> Gateway[API gateway]
    subgraph Services
        Gateway --> Auth{Authorized?}
        Auth -->|Yes| Worker[Process request]
        Auth -->|No| Reject[Return 401]
        Worker --> Store[(Database)]
    end
    Store --> Response([Response])
```

## Sequence

```mermaid
sequenceDiagram
    actor User
    participant API
    participant Cache
    participant DB as Database
    User->>API: Fetch profile
    API->>Cache: Lookup profile
    alt Cache hit
        Cache-->>API: Cached profile
    else Cache miss
        API->>DB: Read profile
        DB-->>API: Profile
        API->>Cache: Store profile
    end
    API-->>User: Return profile
```

## State machine

```mermaid
stateDiagram-v2
    [*] --> Draft
    Draft --> Review: Submit
    Review --> Published: Approve
    Review --> Draft: Request changes
    Published --> Archived: Retire
    Archived --> [*]
```

## Classes

```mermaid
classDiagram
    class Document {
        +String title
        +render()
    }
    class Diagram {
        +String source
        +layout()
    }
    class Renderer {
        +renderPNG()
        +renderUnicode()
    }
    Document "1" *-- "many" Diagram : contains
    Renderer --> Diagram : renders
```

## Relationships

```mermaid
erDiagram
    USER ||--o{ DOCUMENT : owns
    DOCUMENT ||--o{ DIAGRAM : contains
    USER {
        int id PK
        string name
    }
    DOCUMENT {
        int id PK
        string title
    }
    DIAGRAM {
        int id PK
        string source
    }
```

## Distribution

```mermaid
pie title Preview requests
    "Flowcharts" : 45
    "Sequences" : 30
    "States" : 15
    "Other" : 10
```
