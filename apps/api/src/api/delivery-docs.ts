/** Streaming routes bypass HttpApi's JSON encoding, so their binary contracts live here. */
export const deliveryPaths = {
  "/api/v1/source-assets": {
    get: {
      operationId: "sourceAsset",
      description:
        "Load a cover/page URL returned by a source query. Tokens are opaque, require normal API authorization and expire within ten minutes. Requery on 410. Bytes are fetched on demand, capped at 32 MiB and checked for an image signature.",
      parameters: [{ name: "token", in: "query", required: true, schema: { type: "string" } }],
      responses: {
        "200": {
          description: "Image bytes with a content-hash ETag",
          content: { "image/*": { schema: { type: "string", format: "binary" } } },
        },
        "304": { description: "If-None-Match matched the image ETag" },
        "410": { description: "Invalid, missing or expired token; query the source again" },
        "502": { description: "Upstream image unavailable or invalid" },
      },
    },
  },
  "/api/v1/assets/{hash}": {
    get: {
      operationId: "originalAsset",
      description:
        "Download an imported immutable original. Supports a single byte range, If-Range and If-None-Match. Hash is the publication's assetHash. Content is delivered as an attachment, not parsed reader output.",
      parameters: [
        {
          name: "hash",
          in: "path",
          required: true,
          schema: { type: "string", pattern: "^[a-f0-9]{64}$" },
        },
      ],
      responses: {
        "200": {
          description: "Original bytes",
          content: { "application/octet-stream": { schema: { type: "string", format: "binary" } } },
        },
        "206": { description: "Requested byte range" },
        "304": { description: "ETag matched" },
        "404": { description: "Unknown asset hash" },
        "416": { description: "Invalid or unsatisfiable byte range" },
        "503": { description: "Stored bytes unavailable or inconsistent with the catalog" },
      },
    },
  },
}
