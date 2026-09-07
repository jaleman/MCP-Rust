# Media Registry Design Spec

## Goal

Support searchable media resources alongside the text knowledge bundle without mixing binary assets into the document extraction pipeline.

The repository already treats KUKA source documents and extracted knowledge as separate layers:

- `kuka-docs/` = local source documents
- `knowledge/` = extracted markdown bundle

This design adds a third layer:

- `kuka-movies/` = media assets such as `.mov`, `.mp4`
- `kuka-prints/` = media assets such as `.pdf` print sheets
- a media registry that indexes those assets by topic and keyword

## Design principles

1. Media files are not ingested into the text knowledge bundle.
2. Media files are searchable by topic keywords, not by full-text extraction.
3. Each media item gets metadata describing context, media type, and topic.
4. The server exposes a separate media query surface (`list_media`, `get_media`).
5. The registry supports both video and print-type assets with a unified schema.

## Why this is needed

Some useful KUKA assets are not plain text documents and are not a fit for the current extractor:

- video demonstrations
- print sheets and electrical diagrams in PDF format
- training walkthroughs or troubleshooting recordings

These are valuable operational resources, but they do not belong in the markdown knowledge corpus. They need their own search surface keyed by topic such as:

- electrical
- mechanical
- pneumatic
- safety
- vision
- localization
- ecs
- fleet

## Proposed data model

Each media entry should have the following fields:

```json
{
  "id": "cabinet-24v-print",
  "title": "Cabinet 24V Print",
  "type": "print",
  "topic": "electrical",
  "keywords": ["electrical", "24v", "cabinet", "wiring", "control"],
  "mimeType": "application/pdf",
  "folder": "kuka-prints",
  "filename": "cabinet-24v.pdf",
  "uri": "kuka://media/kuka-prints/cabinet-24v.pdf",
  "description": "Electrical print sheet for the 24V cabinet layout"
}
```

And for a video:

```json
{
  "id": "battery-check-demo",
  "title": "Battery Check Demo",
  "type": "video",
  "topic": "electrical",
  "keywords": ["electrical", "battery", "charging", "maintenance"],
  "mimeType": "video/mp4",
  "folder": "kuka-movies",
  "filename": "battery-check-demo.mp4",
  "uri": "kuka://media/kuka-movies/battery-check-demo.mp4",
  "description": "Short operational demo for the battery check workflow"
}
```

Notes:

- `type` describes the asset form: `video`, `print`, `diagram`, `image`, etc.
- `topic` is the primary semantic bucket for search
- `keywords` is a free-form array for search terms and synonyms
- `folder` identifies the asset location in the local workspace
- `uri` is the resource identifier exposed to MCP clients

## Registry location

Suggested registry file(s):

- `kuka-movies/index.json` for video assets
- `kuka-prints/index.json` for print assets

A future consolidation could merge both into one global `media/index.json`, but the initial implementation should keep the registry simple and per-folder to reduce coupling.

## MCP API surface

### list_media

Purpose: list media entries matching a topic or keyword.

Suggested tool signature:

```json
{
  "name": "list_media",
  "description": "List media resources matching a topic or keyword",
  "inputSchema": {
    "type": "object",
    "properties": {
      "topic": { "type": "string" },
      "keyword": { "type": "string" },
      "type": { "type": "string" }
    },
    "additionalProperties": false
  }
}
```

Example queries:

- `list_media(topic="electrical")`
- `list_media(topic="pneumatic")`
- `list_media(keyword="24v")`
- `list_media(type="print", topic="electrical")`

### get_media

Purpose: fetch a single media item by id or keyword and return the resource metadata plus URI.

Suggested tool signature:

```json
{
  "name": "get_media",
  "description": "Fetch a specific media item by id or keyword",
  "inputSchema": {
    "type": "object",
    "properties": {
      "id": { "type": "string" },
      "keyword": { "type": "string" }
    },
    "additionalProperties": false
  }
}
```

## Implementation plan

### Phase 1: define the registry format

- Define the JSON schema for media entries.
- Add `topic`, `keywords`, `type`, `folder`, and `uri` fields.
- Keep the index local and human-editable.
- Keep the registry out of the extracted bundle.

### Phase 2: create the media registries

- Add `kuka-movies/index.json` for current video assets.
- Add `kuka-prints/index.json` for PDF print assets.
- Ensure each file entry has the required metadata and a valid `uri`.

### Phase 3: add media search support to the server

- Add a media registry loader in the Rust server
- Build a small in-memory index keyed by topic and keyword
- Expose `list_media` and `get_media` as MCP tools
- Keep logic separate from `search_docs` and the knowledge bundle index

### Phase 4: support resource access

- Allow MCP clients to access `kuka://media/...` URIs
- Return metadata plus the resource payload when supported by the client
- Preserve the current knowledge bundle behavior without modification

### Phase 5: validation and housekeeping

- Confirm media entries can be listed by topic and keyword
- Confirm files outside the registry are not exposed
- Confirm unsupported media types are handled gracefully
- Add documentation for how to maintain the media indexes

## Validation criteria

The feature is complete when:

- a video file in `kuka-movies/` can be discovered via keyword search
- a print PDF in `kuka-prints/` can be discovered via topic search
- the media index works independently of the extracted knowledge bundle
- the media registry remains easy to maintain by hand for local asset libraries

## Out of scope for the first pass

- automatic OCR or text extraction of media files
- conversion of media into searchable markdown
- indexing binary file contents for semantic matching
- public hosting or remote streaming of media files

## Summary

This design keeps the current knowledge architecture intact while exposing a second, media-first search surface. Media assets are indexed by topic and keyword, and the server can surface them through MCP without having to treat them as extracted KUKA documents.
