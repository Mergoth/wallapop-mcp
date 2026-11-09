# Multiple-photo publication

The upstream project established this HTTP flow in September 2026. The Rust
implementation preserves it in `src/client/publishing.rs`, with local regression
tests in `tests/api.rs`.

1. Validate every local JPEG before making any account request.
2. Read `/api/v3/users/me` for the seller location. If shipping is enabled, obtain
   the account's cost configuration from the explicit input or owned listings.
3. Create the listing with `POST /api/v3/items`, using multipart fields `image`
   (complete JPEG bytes) and `item` (JSON), plus upload-v2 Accept headers.
4. Retain the created item ID immediately.
5. Add each remaining photo with `POST /api/v3/items/{id}/picture2`, multipart
   fields `image` and `order` (2, 3, …). A successful response can contain a
   plain-text picture ID rather than JSON.
6. Return completed/failed photo counts and positions. Never recreate the listing
   when an extra upload fails. Uncertain outcomes are explicitly marked.

Limits: 1–10 JPEGs, 8 MiB per file and 32 MiB total. Files must be regular files;
nonregular files and symlinks are rejected before reading. All file reads are
bounded. Photos are loaded before creation so a missing later file cannot cause
an avoidable partial listing.

Editing uses a complete `PUT /api/v3/items/{id}` payload. Existing picture IDs
and ordering, item location, attributes and shipping consent are retained. An
unknown prerequisite field causes an error rather than an inferred replacement.

The prior browser wizard and Python scripts are no longer part of this project.
The historical endpoint evidence remains in `docs/ENDPOINTS.md` and Git history;
local fixtures do not establish current live compatibility.
